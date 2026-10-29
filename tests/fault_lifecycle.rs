//! Deterministic router-level lifecycle checks complement the real TCP fault
//! tests. A mock connection intentionally never reports a socket flush.
use std::{
    future::poll_fn,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, Version, header},
    response::Response,
};
use http_body::Body as HttpBody;
use serde_json::{Value, json};
use sleepapi::{Config, router, transport::FaultConnection};
use tower::ServiceExt;

fn app(capacity: usize) -> Router {
    router(Config {
        max_in_flight: capacity,
        ..Config::default()
    })
    .unwrap()
}

fn request(method: &str, path: &str, value: Option<Value>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .version(Version::HTTP_11)
        .header(header::CONTENT_TYPE, "application/json")
        .body(value.map_or_else(Body::empty, |value| Body::from(value.to_string())))
        .unwrap()
}

fn attempt(id: &str, generation: u64) -> Request<Body> {
    let mut request = request(
        "POST",
        &format!("/runs/{id}/attempts"),
        Some(json!({"generation":generation})),
    );
    request
        .extensions_mut()
        .insert(ConnectInfo(FaultConnection::default()));
    request
}

async fn json_body(response: Response) -> Value {
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

async fn create(app: &Router, sequence: Value, ttl_ms: u64) -> String {
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/runs",
            Some(json!({"sequence":sequence,"ttl_ms":ttl_ms})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["run_id"].as_str().unwrap().into()
}

async fn receipts(app: &Router, id: &str) -> Value {
    let response = app
        .clone()
        .oneshot(request("GET", &format!("/runs/{id}/receipts"), None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

fn fault_step(truncate: bool, method: &str) -> Value {
    if truncate {
        json!({"status_code":503,"duration_ms":0,"method":method,
            "fixture":{"body":"prefix and remainder"},
            "fault":{"type":"truncated_response","after_bytes":6}})
    } else {
        json!({"status_code":503,"duration_ms":0,"method":method,
            "fault":{"type":"connection_drop"}})
    }
}

fn success_step() -> Value {
    json!({"status_code":200,"duration_ms":0})
}

async fn ordinary(app: &Router, status: StatusCode) {
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/sleep/tokio",
            Some(json!({"duration_ms":0})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    if status == StatusCode::TOO_MANY_REQUESTS {
        assert_eq!(
            json_body(response).await["error"]["code"],
            "capacity_exceeded"
        );
    }
}

async fn finalize(response: Response, inject: bool) {
    if inject {
        assert!(to_bytes(response.into_body(), 8192).await.is_err());
    } else {
        drop(response);
    }
}

fn assert_pending(receipt: &Value) {
    assert_eq!(receipt["outcome"], "pending");
    assert!(receipt["elapsed_ms"].is_null());
    assert!(receipt.get("injection_stage").is_none());
    assert!(receipt.get("injected_body_bytes").is_none());
}

#[tokio::test]
async fn an_unpolled_fault_body_keeps_capacity_and_drop_records_cancellation() {
    for method in ["tokio_sleep", "tokio_spawn", "thread_sleep", "thread_park"] {
        for truncate in [false, true] {
            let app = app(1);
            let id = create(
                &app,
                json!([fault_step(truncate, method), success_step()]),
                300_000,
            )
            .await;
            let response = app.clone().oneshot(attempt(&id, 1)).await.unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            let records = receipts(&app, &id).await;
            assert_pending(&records["receipts"][0]);
            ordinary(&app, StatusCode::TOO_MANY_REQUESTS).await;
            assert_eq!(
                app.clone()
                    .oneshot(request("GET", "/health", None))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
            drop(response);
            let records = receipts(&app, &id).await;
            assert_eq!(records["receipts"][0]["outcome"], "handler_cancelled");
            assert!(records["receipts"][0]["injection_stage"].is_null());
            assert!(records["receipts"][0]["injected_body_bytes"].is_null());
            ordinary(&app, StatusCode::OK).await;
            let next = json_body(app.clone().oneshot(attempt(&id, 1)).await.unwrap()).await;
            assert_eq!(next["attempt"], 2);
            assert_eq!(next["status_code"], 200);
        }
    }
}

#[tokio::test]
async fn polling_connection_drop_records_injection_and_releases_capacity() {
    let app = app(1);
    let id = create(
        &app,
        json!([fault_step(false, "tokio_sleep"), success_step()]),
        300_000,
    )
    .await;
    let response = app.clone().oneshot(attempt(&id, 1)).await.unwrap();
    assert_pending(&receipts(&app, &id).await["receipts"][0]);
    ordinary(&app, StatusCode::TOO_MANY_REQUESTS).await;
    finalize(response, true).await;
    let records = receipts(&app, &id).await;
    let receipt = &records["receipts"][0];
    assert_eq!(receipt["outcome"], "fault_injected");
    assert_eq!(receipt["selected_status_code"], 503);
    assert_eq!(receipt["fault"]["type"], "connection_drop");
    assert_eq!(receipt["injection_stage"], "before_headers");
    assert_eq!(receipt["injected_body_bytes"], 0);
    assert!(receipt["elapsed_ms"].is_number());
    ordinary(&app, StatusCode::OK).await;
    let next = json_body(app.oneshot(attempt(&id, 1)).await.unwrap()).await;
    assert_eq!(next["attempt"], 2);
}

#[tokio::test]
async fn a_prefix_waiting_for_flush_stays_pending_and_cancels_when_dropped() {
    let app = app(1);
    let id = create(&app, json!([fault_step(true, "tokio_sleep")]), 300_000).await;
    let response = app.clone().oneshot(attempt(&id, 1)).await.unwrap();
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "20");
    let mut body = response.into_body();
    let frame = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.into_data().unwrap().as_ref(), b"prefix");
    let mut cx = Context::from_waker(Waker::noop());
    assert!(matches!(
        Pin::new(&mut body).poll_frame(&mut cx),
        Poll::Pending
    ));
    assert_pending(&receipts(&app, &id).await["receipts"][0]);
    ordinary(&app, StatusCode::TOO_MANY_REQUESTS).await;
    drop(body);
    let records = receipts(&app, &id).await;
    assert_eq!(records["receipts"][0]["outcome"], "handler_cancelled");
    assert!(records["receipts"][0].get("injection_stage").is_none());
    ordinary(&app, StatusCode::OK).await;
}

#[tokio::test]
async fn late_fault_injection_or_drop_cannot_modify_a_reset_generations_receipt() {
    for inject in [false, true] {
        let app = app(2);
        let id = create(&app, json!([fault_step(false, "tokio_sleep")]), 300_000).await;
        let old = app.clone().oneshot(attempt(&id, 1)).await.unwrap();
        let reset = app
            .clone()
            .oneshot(request(
                "POST",
                &format!("/runs/{id}/reset"),
                Some(json!({"generation":1})),
            ))
            .await
            .unwrap();
        assert_eq!(reset.status(), StatusCode::OK);
        assert_eq!(json_body(reset).await["generation"], 2);
        let current = app.clone().oneshot(attempt(&id, 2)).await.unwrap();
        ordinary(&app, StatusCode::TOO_MANY_REQUESTS).await;
        let before = receipts(&app, &id).await;
        assert_eq!(before["receipts"].as_array().unwrap().len(), 1);
        assert_pending(&before["receipts"][0]);
        finalize(old, inject).await;
        assert_eq!(receipts(&app, &id).await, before);
        ordinary(&app, StatusCode::OK).await;
        finalize(current, true).await;
        let after = receipts(&app, &id).await;
        assert_eq!(after["receipts"].as_array().unwrap().len(), 1);
        assert_eq!(after["receipts"][0]["generation"], 2);
        assert_eq!(after["receipts"][0]["outcome"], "fault_injected");
    }
}

#[tokio::test(start_paused = true)]
async fn deletion_or_expiry_during_a_fault_body_cannot_resurrect_state() {
    for expire in [false, true] {
        for inject in [false, true] {
            let app = app(1);
            let id = create(&app, json!([fault_step(false, "tokio_sleep")]), 100).await;
            let old = app.clone().oneshot(attempt(&id, 1)).await.unwrap();
            if expire {
                tokio::time::advance(Duration::from_millis(100)).await;
            } else {
                let deleted = app
                    .clone()
                    .oneshot(request("DELETE", &format!("/runs/{id}"), None))
                    .await
                    .unwrap();
                assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
            }
            assert_eq!(
                app.clone()
                    .oneshot(request("GET", &format!("/runs/{id}"), None))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
            let replacement = create(&app, json!([success_step()]), 300_000).await;
            assert_ne!(replacement, id);
            ordinary(&app, StatusCode::TOO_MANY_REQUESTS).await;
            finalize(old, inject).await;
            assert_eq!(
                app.clone()
                    .oneshot(request("GET", &format!("/runs/{id}/receipts"), None))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
            assert!(
                receipts(&app, &replacement).await["receipts"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            let response = app.clone().oneshot(attempt(&replacement, 1)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(json_body(response).await["attempt"], 1);
            let limits = json_body(app.oneshot(request("GET", "/runs", None)).await.unwrap()).await;
            assert_eq!(limits["active_runs"], 1);
        }
    }
}

#[tokio::test]
async fn late_injection_or_drop_does_not_reinsert_an_evicted_fault_receipt() {
    for inject in [false, true] {
        let app = app(2);
        let mut sequence = vec![fault_step(false, "tokio_sleep")];
        sequence.extend(std::iter::repeat_n(success_step(), 32));
        let id = create(&app, json!(sequence), 300_000).await;
        let old = app.clone().oneshot(attempt(&id, 1)).await.unwrap();
        for expected in 2..=33 {
            let response = app.clone().oneshot(attempt(&id, 1)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(json_body(response).await["attempt"], expected);
        }
        let before = receipts(&app, &id).await;
        let records = before["receipts"].as_array().unwrap();
        assert_eq!(records.len(), 32);
        assert_eq!(records.first().unwrap()["attempt"], 2);
        assert_eq!(records.last().unwrap()["attempt"], 33);
        finalize(old, inject).await;
        assert_eq!(receipts(&app, &id).await, before);
    }
}
