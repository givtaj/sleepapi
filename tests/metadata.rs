use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, Version, header},
    response::Response,
};
use serde_json::{Value, json};
use sleepapi::{Config, router, transport::FaultConnection};
use tower::ServiceExt;

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

fn post(path: &str, value: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

async fn body(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn read(app: &Router, path: &str) -> Value {
    let response = app.clone().oneshot(get(path)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body(response).await
}

#[tokio::test]
async fn metadata_is_bounded_and_exposes_only_the_declared_schema() {
    let app = router(Config {
        bind_addr: "192.0.2.42:19999".parse().unwrap(),
        cors_allowed_origins: vec!["https://synthetic-private.example".into()],
        max_duration_ms: 3_600_000,
        max_in_flight: 1024,
        shutdown_grace_ms: 60_000,
    })
    .unwrap();
    let response = app.oneshot(get("/metadata")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(bytes.len() < 4096);
    let metadata: Value = serde_json::from_slice(&bytes).unwrap();
    // Exact allowlist at every level catches accidental configuration, environment,
    // run data, host paths, or extra identity claims being serialized.
    assert_eq!(
        metadata,
        json!({
            "schema_version": 1,
            "contract_version": 1,
            "software": {"name": "sleepapi", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {
                "delays": true, "status_codes": true, "recovery_runs": true,
                "run_reset": true, "run_receipts": true, "response_fixtures": true,
                "connection_drop": false, "truncated_response": false, "persistent_runs": false,
            },
            "protocols": {"transport_faults": ["HTTP/1.1"]},
            "wait_methods": ["tokio_sleep", "tokio_spawn", "thread_sleep", "thread_park"],
            "scenarios": ["fast_success", "slow_provider", "delayed_failure", "client_timeout", "rate_limited", "invalid_request", "upstream_timeout"],
            "limits": {
                "max_duration_ms": 3_600_000, "max_in_flight": 1024, "max_request_body_bytes": 8192,
                "runs": {"max_runs": 64, "max_sequence_steps": 64, "max_attempts_per_run": 1024,
                    "max_receipts_per_run": 32, "default_ttl_ms": 300_000, "max_ttl_ms": 3_600_000},
                "fixtures": {"max_body_bytes": 4096, "max_headers": 5, "max_header_value_bytes": 256,
                    "content_types": ["application/json", "text/plain", "application/xml"],
                    "header_names": ["retry-after", "www-authenticate", "allow", "cache-control", "x-request-id"]},
            },
        })
    );
}

#[tokio::test(start_paused = true)]
async fn metadata_does_not_change_health_runs_or_receipts() {
    let app = router(Config::default()).unwrap();
    let created = app
        .clone()
        .oneshot(post(
            "/runs",
            json!({"sequence":[{"status_code":503,"duration_ms":0}]}),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let run = body(created).await;
    let path = format!("/runs/{}", run["run_id"].as_str().unwrap());
    let response = app
        .clone()
        .oneshot(post(&format!("{path}/attempts"), json!({"generation":1})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let before = read(&app, &path).await;
    let receipts = read(&app, &format!("{path}/receipts")).await;
    let runs = read(&app, "/runs").await;
    let first = read(&app, "/metadata").await;
    assert_eq!(read(&app, "/metadata").await, first);
    assert_eq!(read(&app, "/health").await, json!({"status":"ok"}));
    assert_eq!(read(&app, &path).await, before);
    assert_eq!(read(&app, &format!("{path}/receipts")).await, receipts);
    assert_eq!(read(&app, "/runs").await, runs);
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/metadata")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
    let head = app
        .oneshot(
            Request::builder()
                .method("HEAD")
                .uri("/metadata")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(head.status(), StatusCode::OK);
    assert!(to_bytes(head.into_body(), 4096).await.unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn metadata_uses_effective_limits_without_consuming_simulation_capacity() {
    let app = router(Config {
        max_duration_ms: 17,
        max_in_flight: 1,
        ..Config::default()
    })
    .unwrap();
    let mut waiting = Box::pin(
        app.clone()
            .oneshot(post("/sleep", json!({"duration_ms":17}))),
    );
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    let metadata = read(&app, "/metadata").await;
    assert_eq!(metadata["limits"]["max_duration_ms"], 17);
    assert_eq!(metadata["limits"]["max_in_flight"], 1);
    let rejected = app
        .clone()
        .oneshot(post("/sleep", json!({"duration_ms":0})))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(waiting.await.unwrap().status(), StatusCode::OK);
    let too_long = metadata["limits"]["max_duration_ms"].as_u64().unwrap() + 1;
    for path in ["/sleep", "/runs"] {
        let step = json!({"status_code":200,"duration_ms":too_long});
        let payload = if path == "/runs" {
            json!({"sequence":[step]})
        } else {
            step
        };
        assert_eq!(
            app.clone()
                .oneshot(post(path, payload))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

#[tokio::test]
async fn metadata_discovery_matches_existing_contract_and_validation() {
    let app = router(Config::default()).unwrap();
    let metadata = read(&app, "/metadata").await;
    let methods = read(&app, "/methods").await;
    let scenarios = read(&app, "/scenarios").await;
    assert_eq!(
        metadata["wait_methods"],
        Value::Array(
            methods["methods"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["method"].clone())
                .collect()
        )
    );
    assert_eq!(
        metadata["scenarios"],
        Value::Array(
            scenarios["scenarios"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["scenario"].clone())
                .collect()
        )
    );
    assert_eq!(
        metadata["limits"]["max_duration_ms"],
        methods["limits"]["max_duration_ms"]
    );
    assert_eq!(
        metadata["limits"]["max_in_flight"],
        methods["limits"]["max_in_flight"]
    );
    let limits = &metadata["limits"]["runs"];
    assert_eq!(*limits, read(&app, "/runs").await["limits"]);
    let max_steps = limits["max_sequence_steps"].as_u64().unwrap() as usize;
    let step = json!({"status_code":200,"duration_ms":0});
    for (count, status) in [
        (max_steps, StatusCode::CREATED),
        (max_steps + 1, StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(post("/runs", json!({"sequence": vec![step.clone();count]})))
                .await
                .unwrap()
                .status(),
            status
        );
    }
    let ttl = limits["max_ttl_ms"].as_u64().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(post("/runs", json!({"sequence":[step],"ttl_ms":ttl+1})))
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let max_body = metadata["limits"]["max_request_body_bytes"]
        .as_u64()
        .unwrap() as usize;
    let oversized = Request::builder()
        .method("POST")
        .uri("/runs")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(" ".repeat(max_body + 1)))
        .unwrap();
    assert_eq!(
        app.oneshot(oversized).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn advertised_fixture_constraints_are_enforced() {
    let app = router(Config::default()).unwrap();
    let metadata = read(&app, "/metadata").await;
    let limits = &metadata["limits"]["fixtures"];
    let max_body = limits["max_body_bytes"].as_u64().unwrap() as usize;
    let max_value = limits["max_header_value_bytes"].as_u64().unwrap() as usize;
    let headers: serde_json::Map<String, Value> = limits["header_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| {
            (
                name.as_str().unwrap().to_owned(),
                json!("x".repeat(max_value)),
            )
        })
        .collect();
    assert_eq!(
        headers.len(),
        limits["max_headers"].as_u64().unwrap() as usize
    );
    for content_type in limits["content_types"].as_array().unwrap() {
        for suffix in ["", "; charset=utf-8"] {
            let fixture = json!({"body":"x".repeat(max_body),"content_type":format!("{}{suffix}",content_type.as_str().unwrap()),"headers":headers});
            let response = app
                .clone()
                .oneshot(post(
                    "/runs",
                    json!({"sequence":[{"status_code":200,"fixture":fixture}]}),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
        }
    }
    for fixture in [
        json!({"body":"x".repeat(max_body+1)}),
        json!({"body":"","headers":{"x-request-id":"x".repeat(max_value+1)}}),
        json!({"body":"","headers":{"set-cookie":"synthetic=1"}}),
        json!({"body":"","content_type":"text/html"}),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(post(
                    "/runs",
                    json!({"sequence":[{"status_code":200,"fixture":fixture}]})
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

#[tokio::test]
async fn fault_capabilities_follow_request_protocol_and_listener_context() {
    let app = router(Config::default()).unwrap();
    for (version, instrumented, expected) in [
        (Version::HTTP_11, true, true),
        (Version::HTTP_11, false, false),
        (Version::HTTP_10, true, false),
        (Version::HTTP_2, true, false),
    ] {
        let mut request = get("/metadata");
        *request.version_mut() = version;
        if instrumented {
            request
                .extensions_mut()
                .insert(ConnectInfo(FaultConnection::default()));
        }
        let metadata = body(app.clone().oneshot(request).await.unwrap()).await;
        assert_eq!(metadata["capabilities"]["connection_drop"], expected);
        assert_eq!(metadata["capabilities"]["truncated_response"], expected);
        assert_eq!(
            metadata["protocols"]["transport_faults"],
            json!(["HTTP/1.1"])
        );
    }
}
