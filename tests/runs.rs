use std::{
    collections::BTreeSet,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    response::Response,
};
use serde_json::{Value, json};
use sleepapi::{Config, router};
use tower::ServiceExt;

fn app() -> Router {
    router(Config::default()).unwrap()
}

fn post(path: &str, body: Value) -> Request<Body> {
    raw_post(path, body.to_string(), true)
}

fn raw_post(path: &str, body: String, content_type: bool) -> Request<Body> {
    let mut request = Request::builder().method("POST").uri(path);
    if content_type {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    request.body(Body::from(body)).unwrap()
}

fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

fn attempt(id: &str, generation: u64) -> Request<Body> {
    post(
        &format!("/runs/{id}/attempts"),
        json!({"generation": generation}),
    )
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

async fn body(response: Response) -> Value {
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

async fn api_error(response: Response, status: StatusCode) -> Value {
    assert_eq!(response.status(), status);
    let result = body(response).await;
    assert_eq!(result["simulated"], false, "{result}");
    assert_eq!(result["status_code"], status.as_u16());
    assert!(!result["error"]["code"].as_str().unwrap().is_empty());
    assert!(!result["error"]["message"].as_str().unwrap().is_empty());
    assert!(result["error"]["retryable"].is_boolean());
    result
}

async fn create(app: &Router, value: Value) -> Value {
    let response = app.clone().oneshot(post("/runs", value)).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    body(response).await
}

async fn inspect(app: &Router, id: &str) -> Value {
    let response = app
        .clone()
        .oneshot(request("GET", &format!("/runs/{id}")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body(response).await
}

async fn receipts(app: &Router, id: &str) -> Value {
    let response = app
        .clone()
        .oneshot(request("GET", &format!("/runs/{id}/receipts")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body(response).await
}

async fn reset(app: &Router, id: &str, generation: u64) -> Value {
    let response = app
        .clone()
        .oneshot(post(
            &format!("/runs/{id}/reset"),
            json!({"generation": generation}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body(response).await
}

fn zero_steps(statuses: &[u16]) -> Value {
    json!({"sequence": statuses.iter().map(|status| json!({"status_code": status, "duration_ms": 0})).collect::<Vec<_>>()})
}

#[tokio::test(start_paused = true)]
async fn run_limits_defaults_and_creation_are_discoverable() {
    let app = app();
    let discovery = body(app.clone().oneshot(request("GET", "/runs")).await.unwrap()).await;
    assert_eq!(discovery["active_runs"], 0);
    assert_eq!(
        discovery["limits"],
        json!({"max_runs":64,"max_sequence_steps":64,"max_attempts_per_run":1024,
            "max_receipts_per_run":32,"default_ttl_ms":300000,"max_ttl_ms":3600000})
    );
    let run = create(&app, json!({"sequence":[{"status_code":503}]})).await;
    let id = run["run_id"].as_str().unwrap();
    assert!(!id.is_empty());
    assert_eq!(run["generation"], 1);
    assert_eq!(run["attempts_started"], 0);
    assert_eq!(run["generation_attempts"], 0);
    assert_eq!(run["ttl_ms"], 300_000);
    assert_eq!(run["expires_in_ms"], 300_000);
    assert_eq!(run["retained_receipts"], 0);
    assert_eq!(run["sequence"][0]["duration_ms"], 400);
    assert_eq!(run["sequence"][0]["method"], "tokio_sleep");
    assert_eq!(inspect(&app, id).await, run);
    let discovered = body(app.oneshot(request("GET", "/runs")).await.unwrap()).await;
    assert_eq!(discovered["active_runs"], 1);
}

#[tokio::test(start_paused = true)]
async fn recovery_sequences_are_isolated_finite_and_resettable() {
    let app = app();
    let first = create(&app, zero_steps(&[503, 503, 200])).await;
    let second = create(&app, zero_steps(&[503, 503, 200])).await;
    let ids = [
        first["run_id"].as_str().unwrap(),
        second["run_id"].as_str().unwrap(),
    ];
    assert_ne!(ids[0], ids[1]);
    for (index, expected) in [503, 503, 200].into_iter().enumerate() {
        for id in ids {
            let response = app.clone().oneshot(attempt(id, 1)).await.unwrap();
            assert_eq!(response.status().as_u16(), expected);
            if expected == 503 {
                assert_eq!(response.headers()[header::RETRY_AFTER], "1");
            }
            let result = body(response).await;
            assert_eq!(result["run_id"], id);
            assert_eq!(result["generation"], 1);
            assert_eq!(result["attempt"], index + 1);
            assert_eq!(result["sequence_step"], index + 1);
            assert_eq!(result["operation"], "run");
            assert_eq!(result["simulated"], true);
            assert_eq!(result["status_code"], expected);
            if expected == 503 {
                assert_eq!(result["error"]["code"], "service_unavailable");
            }
        }
    }
    for id in ids {
        api_error(
            app.clone().oneshot(attempt(id, 1)).await.unwrap(),
            StatusCode::CONFLICT,
        )
        .await;
        assert_eq!(inspect(&app, id).await["attempts_started"], 3);
    }
    let reset = reset(&app, ids[0], 1).await;
    assert_eq!(reset["generation"], 2);
    assert_eq!(reset["attempts_started"], 3);
    assert_eq!(reset["generation_attempts"], 0);
    assert_eq!(reset["retained_receipts"], 0);
    assert!(
        receipts(&app, ids[0]).await["receipts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(inspect(&app, ids[1]).await["generation"], 1);
    let response = app.clone().oneshot(attempt(ids[0], 2)).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let result = body(response).await;
    assert_eq!(result["attempt"], 1);
    assert_eq!(result["generation"], 2);
    assert_eq!(inspect(&app, ids[0]).await["attempts_started"], 4);
}

#[tokio::test(start_paused = true)]
async fn sequence_assignment_does_not_follow_completion_order() {
    let app = app();
    let run = create(
        &app,
        json!({"sequence":[
            {"status_code":503,"duration_ms":100},
            {"status_code":200,"duration_ms":10}
        ]}),
    )
    .await;
    let id = run["run_id"].as_str().unwrap();
    let mut first = Box::pin(app.clone().oneshot(attempt(id, 1)));
    assert!(poll_once(first.as_mut()).is_pending());
    let mut second = Box::pin(app.clone().oneshot(attempt(id, 1)));
    assert!(poll_once(second.as_mut()).is_pending());
    let pending = receipts(&app, id).await;
    assert_eq!(pending["receipts"][0]["outcome"], "pending");
    assert_eq!(pending["receipts"][1]["outcome"], "pending");
    tokio::time::advance(Duration::from_millis(10)).await;
    let second = body(second.await.unwrap()).await;
    assert_eq!(second["attempt"], 2);
    assert_eq!(second["sequence_step"], 2);
    assert_eq!(second["status_code"], 200);
    assert!(poll_once(first.as_mut()).is_pending());
    tokio::time::advance(Duration::from_millis(90)).await;
    let first = body(first.await.unwrap()).await;
    assert_eq!(first["attempt"], 1);
    assert_eq!(first["status_code"], 503);
    let completed = receipts(&app, id).await;
    for (i, status) in [503, 200].into_iter().enumerate() {
        let receipt = &completed["receipts"][i];
        assert_eq!(receipt["attempt"], i + 1);
        assert_eq!(receipt["selected_status_code"], status);
        assert_eq!(receipt["outcome"], "completed");
        assert!(receipt["elapsed_ms"].as_f64().unwrap() >= 0.0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_attempts_receive_unique_contiguous_positions() {
    let app = app();
    let run = create(&app, zero_steps(&[200; 32])).await;
    let id = run["run_id"].as_str().unwrap();
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let app = app.clone();
        let request = attempt(id, 1);
        requests.spawn(async move { body(app.oneshot(request).await.unwrap()).await });
    }
    let mut assigned = BTreeSet::new();
    while let Some(result) = requests.join_next().await {
        let result = result.unwrap();
        assert_eq!(result["status_code"], 200);
        assert_eq!(result["attempt"], result["sequence_step"]);
        assert!(assigned.insert(result["attempt"].as_u64().unwrap()));
    }
    assert_eq!(assigned, (1..=32).collect());
    assert_eq!(inspect(&app, id).await["attempts_started"], 32);
    assert_eq!(
        receipts(&app, id).await["receipts"]
            .as_array()
            .unwrap()
            .len(),
        32
    );
}

#[tokio::test(start_paused = true)]
async fn invalid_and_stale_requests_do_not_consume_attempts_or_reset_state() {
    let app = app();
    let run = create(&app, zero_steps(&[503, 200])).await;
    let id = run["run_id"].as_str().unwrap();
    let path = format!("/runs/{id}/attempts");
    for value in [
        json!({}),
        json!({"generation":0}),
        json!({"generation":2}),
        json!({"generation":1,"extra":true}),
    ] {
        let response = app
            .clone()
            .oneshot(post(&path, value.clone()))
            .await
            .unwrap();
        let expected = if value == json!({"generation":0}) || value == json!({"generation":2}) {
            StatusCode::CONFLICT
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        };
        api_error(response, expected).await;
    }
    api_error(
        app.clone()
            .oneshot(raw_post(&path, "{".into(), true))
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
    )
    .await;
    api_error(
        app.clone()
            .oneshot(raw_post(&path, "{}".into(), false))
            .await
            .unwrap(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
    )
    .await;
    assert_eq!(inspect(&app, id).await["attempts_started"], 0);
    assert!(
        receipts(&app, id).await["receipts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    reset(&app, id, 1).await;
    api_error(
        app.clone().oneshot(attempt(id, 1)).await.unwrap(),
        StatusCode::CONFLICT,
    )
    .await;
    api_error(
        app.clone()
            .oneshot(post(&format!("/runs/{id}/reset"), json!({"generation":1})))
            .await
            .unwrap(),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(inspect(&app, id).await["generation"], 2);
    let response = body(app.oneshot(attempt(id, 2)).await.unwrap()).await;
    assert_eq!(response["attempt"], 1);
    assert_eq!(response["status_code"], 503);
}

#[tokio::test(start_paused = true)]
async fn run_attempts_share_capacity_with_stateless_simulations() {
    let app = router(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .unwrap();
    let run = create(&app, zero_steps(&[503])).await;
    let id = run["run_id"].as_str().unwrap();
    let mut ordinary = Box::pin(
        app.clone()
            .oneshot(post("/sleep", json!({"duration_ms":1000}))),
    );
    assert!(poll_once(ordinary.as_mut()).is_pending());
    let rejected = api_error(
        app.clone().oneshot(attempt(id, 1)).await.unwrap(),
        StatusCode::TOO_MANY_REQUESTS,
    )
    .await;
    assert_eq!(rejected["error"]["code"], "capacity_exceeded");
    assert_eq!(inspect(&app, id).await["attempts_started"], 0);
    drop(ordinary);
    assert_eq!(
        app.clone().oneshot(attempt(id, 1)).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let slow = create(
        &app,
        json!({"sequence":[{"status_code":200,"duration_ms":1000}]}),
    )
    .await;
    let slow_id = slow["run_id"].as_str().unwrap();
    let mut pending = Box::pin(app.clone().oneshot(attempt(slow_id, 1)));
    assert!(poll_once(pending.as_mut()).is_pending());
    api_error(
        app.clone()
            .oneshot(post("/sleep", json!({"duration_ms":0})))
            .await
            .unwrap(),
        StatusCode::TOO_MANY_REQUESTS,
    )
    .await;
    assert_eq!(
        app.clone()
            .oneshot(request("GET", "/health"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    // Management operations remain available even while the simulation slot is full.
    reset(&app, slow_id, 1).await;
    assert_eq!(inspect(&app, slow_id).await["generation"], 2);
    drop(pending);
    // Reservation rejection happens after acquiring the sole global slot.
    // Neither a stale generation nor an exhausted sequence may leak that slot.
    for generation in [2, 1] {
        api_error(
            app.clone().oneshot(attempt(id, generation)).await.unwrap(),
            StatusCode::CONFLICT,
        )
        .await;
        assert_eq!(
            app.clone()
                .oneshot(post("/sleep", json!({"duration_ms":0})))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
}

#[tokio::test(start_paused = true)]
async fn reset_during_an_attempt_does_not_import_old_completion_or_cancellation() {
    for cancel in [false, true] {
        let app = app();
        let run = create(
            &app,
            json!({"sequence":[{"status_code":503,"duration_ms":100}]}),
        )
        .await;
        let id = run["run_id"].as_str().unwrap();
        let mut pending = Box::pin(app.clone().oneshot(attempt(id, 1)));
        assert!(poll_once(pending.as_mut()).is_pending());
        reset(&app, id, 1).await;
        if cancel {
            drop(pending);
        } else {
            tokio::time::advance(Duration::from_millis(100)).await;
            let result = body(pending.await.unwrap()).await;
            assert_eq!(result["generation"], 1);
            assert_eq!(result["attempt"], 1);
        }
        let state = inspect(&app, id).await;
        assert_eq!(state["generation"], 2);
        assert_eq!(state["attempts_started"], 1);
        assert_eq!(state["generation_attempts"], 0);
        assert!(
            receipts(&app, id).await["receipts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let result = body(app.clone().oneshot(attempt(id, 2)).await.unwrap()).await;
        assert_eq!(result["generation"], 2);
        assert_eq!(result["attempt"], 1);
        let records = receipts(&app, id).await;
        assert_eq!(records["receipts"].as_array().unwrap().len(), 1);
        assert_eq!(records["receipts"][0]["generation"], 2);
        assert_eq!(records["receipts"][0]["outcome"], "completed");
    }
}

#[tokio::test(start_paused = true)]
async fn deletion_and_expiry_during_work_cannot_resurrect_runs() {
    for expire in [false, true] {
        let app = app();
        let run = create(
            &app,
            json!({"sequence":[{"status_code":200,"duration_ms":100}],"ttl_ms":10}),
        )
        .await;
        let id = run["run_id"].as_str().unwrap();
        let mut pending = Box::pin(app.clone().oneshot(attempt(id, 1)));
        assert!(poll_once(pending.as_mut()).is_pending());
        if expire {
            tokio::time::advance(Duration::from_millis(10)).await;
        } else {
            let response = app
                .clone()
                .oneshot(request("DELETE", &format!("/runs/{id}")))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert!(
                to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        api_error(
            app.clone()
                .oneshot(request("GET", &format!("/runs/{id}")))
                .await
                .unwrap(),
            StatusCode::NOT_FOUND,
        )
        .await;
        let replacement = create(&app, zero_steps(&[503])).await;
        let replacement_id = replacement["run_id"].as_str().unwrap();
        assert_ne!(replacement_id, id);
        tokio::time::advance(Duration::from_millis(100)).await;
        let completed = body(pending.await.unwrap()).await;
        assert_eq!(completed["run_id"], id);
        assert_eq!(completed["status_code"], 200);
        api_error(
            app.clone()
                .oneshot(request("GET", &format!("/runs/{id}/receipts")))
                .await
                .unwrap(),
            StatusCode::NOT_FOUND,
        )
        .await;
        assert!(
            receipts(&app, replacement_id).await["receipts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(inspect(&app, replacement_id).await["attempts_started"], 0);
    }
}

#[tokio::test(start_paused = true)]
async fn reset_does_not_extend_expiry_and_expired_generations_are_inaccessible() {
    let app = app();
    let run = create(
        &app,
        json!({"sequence":[{"status_code":200,"duration_ms":0}],"ttl_ms":100}),
    )
    .await;
    let id = run["run_id"].as_str().unwrap();
    tokio::time::advance(Duration::from_millis(40)).await;
    let state = reset(&app, id, 1).await;
    assert_eq!(state["expires_in_ms"], 60);
    tokio::time::advance(Duration::from_millis(60)).await;
    for request in [
        attempt(id, 2),
        post(&format!("/runs/{id}/reset"), json!({"generation":2})),
        request("GET", &format!("/runs/{id}")),
        request("GET", &format!("/runs/{id}/receipts")),
        request("DELETE", &format!("/runs/{id}")),
    ] {
        api_error(
            app.clone().oneshot(request).await.unwrap(),
            StatusCode::NOT_FOUND,
        )
        .await;
    }
    let discovery = body(app.oneshot(request("GET", "/runs")).await.unwrap()).await;
    assert_eq!(discovery["active_runs"], 0);
}

#[tokio::test(start_paused = true)]
async fn cancelling_async_attempts_records_the_consumed_step_and_releases_capacity() {
    for method in ["tokio_sleep", "tokio_spawn"] {
        let app = router(Config {
            max_in_flight: 1,
            ..Config::default()
        })
        .unwrap();
        let run = create(
            &app,
            json!({"sequence":[
                {"status_code":503,"duration_ms":30000,"method":method},
                {"status_code":200,"duration_ms":0,"method":method}
            ]}),
        )
        .await;
        let id = run["run_id"].as_str().unwrap();
        let mut pending = Box::pin(app.clone().oneshot(attempt(id, 1)));
        assert!(poll_once(pending.as_mut()).is_pending());
        tokio::task::yield_now().await;
        drop(pending);
        tokio::task::yield_now().await;
        let cancelled = receipts(&app, id).await;
        assert_eq!(cancelled["receipts"][0]["outcome"], "handler_cancelled");
        assert_eq!(cancelled["receipts"][0]["attempt"], 1);
        let result = body(app.clone().oneshot(attempt(id, 1)).await.unwrap()).await;
        assert_eq!(result["status_code"], 200);
        assert_eq!(result["attempt"], 2);
        assert_eq!(inspect(&app, id).await["attempts_started"], 2);
    }
}

#[tokio::test(start_paused = true)]
async fn retained_receipts_are_bounded_even_when_an_evicted_attempt_finishes_late() {
    let app = app();
    let mut steps = vec![json!({"status_code":200,"duration_ms":0}); 64];
    steps[0]["duration_ms"] = json!(10000);
    let run = create(&app, json!({"sequence":steps})).await;
    let id = run["run_id"].as_str().unwrap();
    let mut first = Box::pin(app.clone().oneshot(attempt(id, 1)));
    assert!(poll_once(first.as_mut()).is_pending());
    for _ in 2..=64 {
        assert_eq!(
            app.clone().oneshot(attempt(id, 1)).await.unwrap().status(),
            StatusCode::OK
        );
    }
    let before = receipts(&app, id).await;
    let records = before["receipts"].as_array().unwrap();
    assert_eq!(records.len(), 32);
    assert_eq!(records.first().unwrap()["attempt"], 33);
    assert_eq!(records.last().unwrap()["attempt"], 64);
    assert_eq!(inspect(&app, id).await["retained_receipts"], 32);
    tokio::time::advance(Duration::from_millis(10000)).await;
    assert_eq!(first.await.unwrap().status(), StatusCode::OK);
    assert_eq!(receipts(&app, id).await, before);
}

#[tokio::test(start_paused = true)]
async fn run_capacity_is_bounded_and_expired_slots_are_reclaimed() {
    let app = app();
    for _ in 0..64 {
        create(
            &app,
            json!({"sequence":[{"status_code":200,"duration_ms":0}],"ttl_ms":1}),
        )
        .await;
    }
    api_error(
        app.clone()
            .oneshot(post("/runs", zero_steps(&[200])))
            .await
            .unwrap(),
        StatusCode::TOO_MANY_REQUESTS,
    )
    .await;
    let full = body(app.clone().oneshot(request("GET", "/runs")).await.unwrap()).await;
    assert_eq!(full["active_runs"], 64);
    tokio::time::advance(Duration::from_millis(1)).await;
    create(&app, zero_steps(&[200])).await;
    let reclaimed = body(app.oneshot(request("GET", "/runs")).await.unwrap()).await;
    assert_eq!(reclaimed["active_runs"], 1);
}

#[tokio::test(start_paused = true)]
async fn lifetime_attempt_budget_survives_resets() {
    let app = app();
    let run = create(&app, zero_steps(&[200; 64])).await;
    let id = run["run_id"].as_str().unwrap();
    for generation in 1..=16 {
        for _ in 0..64 {
            assert_eq!(
                app.clone()
                    .oneshot(attempt(id, generation))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
        }
        reset(&app, id, generation).await;
    }
    api_error(
        app.clone().oneshot(attempt(id, 17)).await.unwrap(),
        StatusCode::CONFLICT,
    )
    .await;
    let state = inspect(&app, id).await;
    assert_eq!(state["attempts_started"], 1024);
    assert_eq!(state["generation_attempts"], 0);
    assert!(
        receipts(&app, id).await["receipts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let fresh = create(&app, zero_steps(&[200])).await;
    assert_eq!(
        app.oneshot(attempt(fresh["run_id"].as_str().unwrap(), 1))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn run_definitions_reject_invalid_sequences_fields_and_limits_without_allocating() {
    let app = router(Config {
        max_duration_ms: 100,
        ..Config::default()
    })
    .unwrap();
    for invalid in [
        json!({"sequence":[]}),
        zero_steps(&[200; 65]),
        json!({"sequence":[{"status_code":200,"duration_ms":101}]}),
        json!({"sequence":[{"status_code":200,"duration_ms":0}],"ttl_ms":0}),
        json!({"sequence":[{"status_code":200,"duration_ms":0}],"ttl_ms":3600001}),
        json!({"sequence":[{"status_code":200,"duration_ms":0}],"extra":1}),
        json!({"sequence":[{"status_code":200,"duration_ms":0,"extra":1}]}),
        json!({"sequence":[{"duration_ms":0}]}),
        json!({"sequence":[{"status_code":200,"duration_ms":0,"method":"wrong"}]}),
        json!({"sequence":[{"status_code":200,"duration_ms":-1}]}),
    ] {
        api_error(
            app.clone().oneshot(post("/runs", invalid)).await.unwrap(),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }
    for invalid_status in [100, 199, 204, 205, 206, 226, 304, 407, 416, 426, 600] {
        api_error(
            app.clone()
                .oneshot(post("/runs", zero_steps(&[invalid_status])))
                .await
                .unwrap(),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }
    // The shared body limit still applies before a run can be allocated.
    api_error(
        app.clone()
            .oneshot(raw_post(
                "/runs",
                format!("{{\"padding\":\"{}\"}}", "x".repeat(8192)),
                true,
            ))
            .await
            .unwrap(),
        StatusCode::PAYLOAD_TOO_LARGE,
    )
    .await;
    let state = body(app.clone().oneshot(request("GET", "/runs")).await.unwrap()).await;
    assert_eq!(state["active_runs"], 0);
    create(
        &app,
        json!({"sequence":[{"status_code":200,"duration_ms":100}],"ttl_ms":3600000}),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn every_wait_method_preserves_delay_health_and_simulated_protocol_headers() {
    for (method, status, expected_header, expected_value) in [
        (
            "tokio_sleep",
            401,
            header::WWW_AUTHENTICATE,
            "Bearer realm=\"sleepapi\"",
        ),
        ("tokio_spawn", 405, header::ALLOW, "GET"),
        ("thread_sleep", 429, header::RETRY_AFTER, "1"),
        ("thread_park", 503, header::RETRY_AFTER, "1"),
    ] {
        let app = router(Config {
            max_in_flight: 1,
            ..Config::default()
        })
        .unwrap();
        let run = create(
            &app,
            json!({"sequence":[{"status_code":status,"duration_ms":20,"method":method}]}),
        )
        .await;
        let id = run["run_id"].as_str().unwrap();
        let started = Instant::now();
        let mut pending = Box::pin(app.clone().oneshot(attempt(id, 1)));
        assert!(poll_once(pending.as_mut()).is_pending());
        assert_eq!(
            app.clone()
                .oneshot(request("GET", "/health"))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let response = pending.await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(20));
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers()[expected_header], expected_value);
        let result = body(response).await;
        assert_eq!(result["method"], method);
        assert_eq!(result["requested_duration_ms"], 20);
        assert_eq!(result["blocks_thread"], method.starts_with("thread_"));
        assert!(result["elapsed_ms"].as_f64().unwrap() >= 20.0);
    }
}

#[tokio::test]
async fn run_management_errors_are_structured_and_delete_works_with_browser_cors() {
    let origin = "http://localhost:5173";
    let app = router(Config {
        cors_allowed_origins: vec![origin.into()],
        ..Config::default()
    })
    .unwrap();
    for missing in [
        request("GET", "/runs/missing"),
        request("GET", "/runs/missing/receipts"),
        request("DELETE", "/runs/missing"),
        attempt("missing", 1),
        post("/runs/missing/reset", json!({"generation":1})),
    ] {
        api_error(
            app.clone().oneshot(missing).await.unwrap(),
            StatusCode::NOT_FOUND,
        )
        .await;
    }
    api_error(
        app.clone().oneshot(request("PUT", "/runs")).await.unwrap(),
        StatusCode::METHOD_NOT_ALLOWED,
    )
    .await;
    let run = create(&app, zero_steps(&[200])).await;
    let id = run["run_id"].as_str().unwrap();
    let path = format!("/runs/{id}");
    let preflight = Request::builder()
        .method("OPTIONS")
        .uri(&path)
        .header(header::ORIGIN, origin)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "DELETE")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(preflight).await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        origin
    );
    assert!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_METHODS]
            .to_str()
            .unwrap()
            .split(',')
            .any(|m| m.trim() == "DELETE")
    );
    let mut delete = request("DELETE", &path);
    delete
        .headers_mut()
        .insert(header::ORIGIN, origin.parse().unwrap());
    let response = app.clone().oneshot(delete).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        origin
    );
    api_error(
        app.oneshot(request("GET", &path)).await.unwrap(),
        StatusCode::NOT_FOUND,
    )
    .await;
}
