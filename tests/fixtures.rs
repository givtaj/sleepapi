use std::time::{Duration, Instant};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    http::{Request, StatusCode, header},
    response::Response,
};
use serde_json::{Value, json};
use sleepapi::{Config, router};
use tower::ServiceExt;

fn app() -> Router {
    router(Config::default()).unwrap()
}

fn raw_post(path: &str, value: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value))
        .unwrap()
}

fn post(path: &str, value: Value) -> Request<Body> {
    raw_post(path, value.to_string())
}

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

fn attempt(id: &str, generation: u64) -> Request<Body> {
    post(
        &format!("/runs/{id}/attempts"),
        json!({"generation":generation}),
    )
}

async fn bytes(response: Response) -> Bytes {
    to_bytes(response.into_body(), 128 * 1024).await.unwrap()
}

async fn json_body(response: Response) -> Value {
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    serde_json::from_slice(&bytes(response).await).unwrap()
}

async fn create(app: &Router, sequence: Value) -> Value {
    let response = app
        .clone()
        .oneshot(post("/runs", json!({"sequence":sequence})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await
}

async fn reject(app: &Router, request: Request<Body>, status: StatusCode) {
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), status);
    let value = json_body(response).await;
    assert_eq!(value["simulated"], false);
    assert_eq!(value["status_code"], status.as_u16());
    assert!(value["error"]["code"].is_string());
}

#[tokio::test]
async fn fixture_returns_exact_decoded_utf8_bytes_without_metadata() {
    let app = app();
    let cases = [
        ("application/json", " {\n \"customer\": \"café 💤\" } \n"),
        ("text/plain; charset=utf-8", "line one\r\n\tline two\0"),
        ("application/xml", "<answer>déjà vu</answer>\n"),
        // A fixture can deliberately contain malformed JSON for client testing.
        ("application/json", "{ broken json"),
        ("text/plain", ""),
    ];
    for (content_type, body) in cases {
        let run = create(
            &app,
            json!([{"status_code":200,"duration_ms":0,
                "fixture":{"body":body,"content_type":content_type}}]),
        )
        .await;
        let id = run["run_id"].as_str().unwrap();
        let response = app.clone().oneshot(attempt(id, 1)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
        assert_eq!(bytes(response).await.as_ref(), body.as_bytes());
    }

    // Escapes in the create request decode once; no quoting or JSON reencoding
    // may be added to the selected fixture's HTTP response.
    let response = app.clone().oneshot(raw_post("/runs", r#"{"sequence":[{"status_code":200,"duration_ms":0,"fixture":{"body":"\u00e9\n\"quoted\""}}]}"#.into())).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let run = json_body(response).await;
    let id = run["run_id"].as_str().unwrap();
    let response = app.oneshot(attempt(id, 1)).await.unwrap();
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(bytes(response).await.as_ref(), "é\n\"quoted\"".as_bytes());
}

#[tokio::test]
async fn fixtures_keep_status_headers_and_allow_bounded_overrides() {
    let app = app();
    for (status, name, default, replacement) in [
        (
            401,
            "www-authenticate",
            "Bearer realm=\"sleepapi\"",
            "Basic realm=\"fixture\"",
        ),
        (405, "allow", "GET", "PUT"),
        (429, "retry-after", "1", "17"),
        (503, "retry-after", "1", "Thu, 29 Oct 2026 12:00:00 GMT"),
    ] {
        let run = create(&app, json!([
            {"status_code":status,"duration_ms":0,"fixture":{"body":"failure"}},
            {"status_code":status,"duration_ms":0,"fixture":{"body":"failure","headers":{
                name.to_ascii_uppercase():replacement,"X-Request-ID":"fixture-123","Cache-Control":"no-store"}}}
        ])).await;
        let id = run["run_id"].as_str().unwrap();
        for expected in [default, replacement] {
            let response = app.clone().oneshot(attempt(id, 1)).await.unwrap();
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(response.headers()[name], expected);
            if expected == replacement {
                assert_eq!(response.headers()["x-request-id"], "fixture-123");
                assert_eq!(response.headers()["cache-control"], "no-store");
            }
            assert_eq!(bytes(response).await.as_ref(), b"failure");
        }
    }
}

#[tokio::test]
async fn fixture_limits_accept_boundaries_and_reject_before_allocating_a_run() {
    let app = app();
    for body in ["x".repeat(4096), "é".repeat(2048)] {
        let run = create(
            &app,
            json!([{"status_code":200,"duration_ms":0,"fixture":{"body":body,
            "headers":{"retry-after":"0","www-authenticate":"Bearer","allow":"GET",
                "cache-control":"no-store","x-request-id":"x".repeat(256)}}}]),
        )
        .await;
        let response = app
            .clone()
            .oneshot(attempt(run["run_id"].as_str().unwrap(), 1))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-request-id"].as_bytes().len(), 256);
        assert_eq!(bytes(response).await.as_ref(), body.as_bytes());
    }
    for fixture in [
        json!({"body":"x".repeat(4097)}),
        json!({"body":"é".repeat(2049)}),
        json!({"body":"","headers":{"x-request-id":"x".repeat(257)}}),
        json!({"body":"","headers":{"retry-after":"1","www-authenticate":"Bearer","allow":"GET",
            "cache-control":"no-store","x-request-id":"id","content-length":"0"}}),
    ] {
        reject(
            &app,
            post(
                "/runs",
                json!({"sequence":[{"status_code":200,"fixture":fixture}]}),
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }
    let limits = json_body(app.oneshot(get("/runs")).await.unwrap()).await;
    assert_eq!(limits["active_runs"], 2);
}

#[tokio::test]
async fn combined_fixture_request_keeps_the_eight_kib_body_limit() {
    let app = app();
    let step = json!({"status_code":200,"fixture":{"body":"x".repeat(4096)}});
    reject(
        &app,
        post("/runs", json!({"sequence":[step.clone(),step]})),
        StatusCode::PAYLOAD_TOO_LARGE,
    )
    .await;
    let limits = json_body(app.oneshot(get("/runs")).await.unwrap()).await;
    assert_eq!(limits["active_runs"], 0);
}

#[tokio::test]
async fn fixture_schema_and_unsafe_headers_reject_without_retaining_state() {
    let app = app();
    let mut fixtures = vec![
        json!({}),
        json!({"body":null}),
        json!({"body":42}),
        json!({"body":"","unexpected":true}),
        json!({"body":"","content_type":null}),
        json!({"body":"","content_type":"text/html"}),
        json!({"body":"","content_type":"text/plain\r\nX-Evil: yes"}),
        json!({"body":"","headers":null}),
        json!({"body":"","headers":{"retry-after":1}}),
    ];
    for name in [
        "Content-Length",
        "Transfer-Encoding",
        "Connection",
        "Content-Type",
        "Access-Control-Allow-Origin",
        "Access-Control-Allow-Credentials",
        "Set-Cookie",
        "allow\n",
        "rétry-after",
    ] {
        fixtures.push(json!({"body":"","headers":{name:"1"}}));
    }
    for value in ["line\r\nbreak", "tab\there", "nul\0here", "déjà", "\u{7f}"] {
        fixtures.push(json!({"body":"","headers":{"x-request-id":value}}));
    }
    for fixture in fixtures {
        reject(
            &app,
            post(
                "/runs",
                json!({"sequence":[{"status_code":200,"fixture":fixture}]}),
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }
    for headers in [
        r#"{"allow":"GET","allow":"POST"}"#,
        r#"{"Allow":"GET","ALLOW":"POST"}"#,
    ] {
        let request = format!(
            r#"{{"sequence":[{{"status_code":200,"fixture":{{"body":"","headers":{headers}}}}}]}}"#
        );
        reject(
            &app,
            raw_post("/runs", request),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }
    let limits = json_body(app.oneshot(get("/runs")).await.unwrap()).await;
    assert_eq!(limits["active_runs"], 0);
}

#[tokio::test]
async fn fixtures_cannot_bypass_supported_status_restrictions() {
    let app = app();
    for status in [100, 199, 204, 205, 206, 226, 304, 407, 416, 426, 600] {
        reject(
            &app,
            post(
                "/runs",
                json!({"sequence":[{"status_code":status,"fixture":{"body":""}}]}),
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }
    let limits = json_body(app.oneshot(get("/runs")).await.unwrap()).await;
    assert_eq!(limits["active_runs"], 0);
}

#[tokio::test]
async fn fixture_diagnostics_stay_separate_and_reset_preserves_run_isolation() {
    let app = app();
    let first = create(
        &app,
        json!([
            {"status_code":503,"duration_ms":0,"fixture":{"body":"retry me"}},
            {"status_code":200,"duration_ms":0,"fixture":{"body":"done"}}
        ]),
    )
    .await;
    let second = create(
        &app,
        json!([{"status_code":200,"duration_ms":0,"fixture":{"body":"other"}}]),
    )
    .await;
    let first_id = first["run_id"].as_str().unwrap();
    let second_id = second["run_id"].as_str().unwrap();
    assert_eq!(
        bytes(app.clone().oneshot(attempt(first_id, 1)).await.unwrap())
            .await
            .as_ref(),
        b"retry me"
    );
    let receipts = json_body(
        app.clone()
            .oneshot(get(&format!("/runs/{first_id}/receipts")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(receipts["run_id"], first_id);
    assert_eq!(receipts["generation"], 1);
    let receipt = &receipts["receipts"][0];
    assert_eq!(receipt["outcome"], "completed");
    assert_eq!(receipt["attempt"], 1);
    assert_eq!(receipt["sequence_step"], 1);
    assert_eq!(receipt["selected_status_code"], 503);
    assert!(receipt["elapsed_ms"].is_number());
    assert!(receipt.get("body").is_none());
    let reset = app
        .clone()
        .oneshot(post(
            &format!("/runs/{first_id}/reset"),
            json!({"generation":1}),
        ))
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::OK);
    assert_eq!(json_body(reset).await["generation"], 2);
    reject(&app, attempt(first_id, 1), StatusCode::CONFLICT).await;
    assert_eq!(
        bytes(app.clone().oneshot(attempt(first_id, 2)).await.unwrap())
            .await
            .as_ref(),
        b"retry me"
    );
    assert_eq!(
        bytes(app.clone().oneshot(attempt(first_id, 2)).await.unwrap())
            .await
            .as_ref(),
        b"done"
    );
    assert_eq!(
        bytes(app.clone().oneshot(attempt(second_id, 1)).await.unwrap())
            .await
            .as_ref(),
        b"other"
    );
    let receipts = json_body(
        app.oneshot(get(&format!("/runs/{first_id}/receipts")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(receipts["receipts"].as_array().unwrap().len(), 2);
    assert_eq!(receipts["receipts"][0]["generation"], 2);
}

#[tokio::test]
async fn every_wait_method_delays_before_returning_the_fixture() {
    let app = app();
    for method in ["tokio_sleep", "tokio_spawn", "thread_sleep", "thread_park"] {
        let run = create(
            &app,
            json!([{"status_code":200,"duration_ms":10,"method":method,
            "fixture":{"body":"ready"}}]),
        )
        .await;
        let started = Instant::now();
        let response = app
            .clone()
            .oneshot(attempt(run["run_id"].as_str().unwrap(), 1))
            .await
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(10));
        assert_eq!(bytes(response).await.as_ref(), b"ready");
    }
}
