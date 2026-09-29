use std::{
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

const METHODS: [(&str, bool); 4] = [
    ("tokio_sleep", false),
    ("tokio_spawn", false),
    ("thread_sleep", true),
    ("thread_park", true),
];

const SCENARIOS: [(&str, u64, u16); 7] = [
    ("fast_success", 50, 200),
    ("slow_provider", 2_000, 200),
    ("delayed_failure", 800, 503),
    ("client_timeout", 3_000, 200),
    ("rate_limited", 100, 429),
    ("invalid_request", 50, 400),
    ("upstream_timeout", 2_000, 504),
];

fn app() -> Router {
    router(Config::default()).unwrap()
}

fn post(path: &str, body: Value) -> Request<Body> {
    raw_post(path, body.to_string(), true)
}

fn raw_post(path: &str, body: String, json_content_type: bool) -> Request<Body> {
    let mut request = Request::builder().method("POST").uri(path);
    if json_content_type {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    request.body(Body::from(body)).unwrap()
}

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

async fn json_body(response: Response) -> Value {
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

fn assert_result(body: &Value, operation: &str, method: &str, duration_ms: u64, status: u16) {
    assert_scenario_result(body, operation, method, duration_ms, status, None);
}

fn assert_scenario_result(
    body: &Value,
    operation: &str,
    method: &str,
    duration_ms: u64,
    status: u16,
    scenario: Option<&str>,
) {
    let expected_fields = 7 + usize::from(status >= 400) + usize::from(scenario.is_some());
    assert_eq!(body.as_object().unwrap().len(), expected_fields, "{body}");
    assert_eq!(body["operation"], operation);
    assert_eq!(body["simulated"], true);
    assert_eq!(body["method"], method);
    assert_eq!(body["requested_duration_ms"], duration_ms);
    assert_eq!(body["status_code"], status);
    assert_eq!(body["blocks_thread"], method.starts_with("thread_"));
    assert!(body["elapsed_ms"].as_f64().unwrap() >= 0.0);
    match scenario {
        Some(scenario) => assert_eq!(body["scenario"], scenario),
        None => assert!(body.get("scenario").is_none(), "{body}"),
    }
    if status >= 400 {
        assert_error_shape(&body["error"]);
    } else {
        assert!(body.get("error").is_none(), "{body}");
    }
}

fn assert_error_shape(error: &Value) {
    assert_eq!(error.as_object().unwrap().len(), 3, "{error}");
    assert!(!error["code"].as_str().unwrap().is_empty(), "{error}");
    assert!(!error["message"].as_str().unwrap().is_empty(), "{error}");
    assert!(error["retryable"].is_boolean(), "{error}");
}

async fn assert_api_error(response: Response) -> Value {
    let status = response.status().as_u16();
    let body = json_body(response).await;
    assert_eq!(body.as_object().unwrap().len(), 3, "{body}");
    assert_eq!(body["simulated"], false);
    assert_eq!(body["status_code"], status);
    assert_error_shape(&body["error"]);
    body
}

fn cors_app() -> Router {
    router(Config {
        cors_allowed_origins: vec!["http://localhost:5173".into()],
        ..Config::default()
    })
    .unwrap()
}

fn preflight(path: &str, origin: &str) -> Request<Body> {
    Request::builder()
        .method("OPTIONS")
        .uri(path)
        .header(header::ORIGIN, origin)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .header(
            header::ACCESS_CONTROL_REQUEST_HEADERS,
            "content-type, authorization",
        )
        .body(Body::empty())
        .unwrap()
}

fn with_origin(mut request: Request<Body>, origin: &str) -> Request<Body> {
    request
        .headers_mut()
        .insert(header::ORIGIN, origin.parse().unwrap());
    request
}

fn assert_header_token(response: &Response, name: &str, expected: &str) {
    assert!(
        response.headers().get_all(name).iter().any(|value| {
            value
                .to_str()
                .unwrap()
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case(expected))
        }),
        "missing {expected} in {name}: {:?}",
        response.headers()
    );
}

#[tokio::test]
async fn cors_is_disabled_by_default() {
    let app = app();
    let preflight = app
        .clone()
        .oneshot(preflight("/sleep/tokio", "http://localhost:5173"))
        .await
        .unwrap();
    assert_eq!(preflight.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        !preflight
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );

    let response = app
        .oneshot(with_origin(
            post("/sleep/tokio", json!({"duration_ms": 0})),
            "http://localhost:5173",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );
}

#[tokio::test]
async fn configured_cors_allows_json_and_authorization_preflights() {
    let app = cors_app();
    for path in [
        "/sleep/tokio",
        "/sleep/tokio-spawn",
        "/sleep/thread",
        "/sleep/park",
        "/sleep",
        "/simulate/email",
        "/simulate/sms",
    ] {
        let response = app
            .clone()
            .oneshot(preflight(path, "http://localhost:5173"))
            .await
            .unwrap();
        assert!(response.status().is_success(), "{path}");
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "http://localhost:5173"
        );
        assert_header_token(&response, "access-control-allow-methods", "POST");
        assert_header_token(&response, "access-control-allow-headers", "Content-Type");
        assert_header_token(&response, "access-control-allow-headers", "Authorization");
    }
}

#[tokio::test]
async fn configured_cors_covers_success_simulated_failures_and_api_rejections() {
    let app = cors_app();
    let cases = [
        (post("/sleep/tokio", json!({"duration_ms": 0})), 200, true),
        (
            post(
                "/sleep/tokio",
                json!({"scenario": "rate_limited", "duration_ms": 0}),
            ),
            429,
            true,
        ),
        (
            post(
                "/sleep/tokio",
                json!({"scenario": "delayed_failure", "duration_ms": 0}),
            ),
            503,
            true,
        ),
        (
            post(
                "/sleep/tokio",
                json!({"duration_ms": 0, "status_code": 401}),
            ),
            401,
            true,
        ),
        (
            post(
                "/sleep/tokio",
                json!({"duration_ms": 0, "status_code": 405}),
            ),
            405,
            true,
        ),
        (post("/sleep/tokio", json!({"duration_ms": -1})), 422, false),
        (raw_post("/sleep/tokio", "{".into(), true), 400, false),
        (get("/sleep/tokio"), 405, false),
        (get("/missing"), 404, false),
    ];
    for (request, status, simulated) in cases {
        let response = app
            .clone()
            .oneshot(with_origin(request, "http://localhost:5173"))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "http://localhost:5173"
        );
        for exposed in ["Retry-After", "WWW-Authenticate", "Allow"] {
            assert_header_token(&response, "access-control-expose-headers", exposed);
        }
        let body = json_body(response).await;
        assert_eq!(body["simulated"], simulated);
        assert_eq!(body["status_code"], status);
    }
}

#[tokio::test]
async fn configured_cors_does_not_allow_other_origins() {
    let app = cors_app();
    for origin in [
        "http://localhost:5174",
        "http://localhost:5173.example.com",
        "null",
    ] {
        for request in [
            preflight("/sleep/tokio", origin),
            with_origin(post("/sleep/tokio", json!({"duration_ms": 0})), origin),
        ] {
            let response = app.clone().oneshot(request).await.unwrap();
            assert!(
                !response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
                "unexpected access for {origin}"
            );
        }
    }
}

#[tokio::test]
async fn health_and_methods_describe_configured_service() {
    let app = router(Config {
        max_duration_ms: 2_000,
        max_in_flight: 3,
        ..Config::default()
    })
    .unwrap();
    let health = app.clone().oneshot(get("/health")).await.unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(json_body(health).await, json!({"status": "ok"}));

    let response = app.oneshot(get("/methods")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["limits"]["max_duration_ms"], 2_000);
    assert_eq!(body["limits"]["max_in_flight"], 3);
    assert_eq!(
        body["defaults"],
        json!({"duration_ms": 400, "method": "tokio_sleep", "status_code": 200})
    );
    let methods = body["methods"].as_array().unwrap();
    assert_eq!(methods.len(), METHODS.len());
    for (name, blocks_thread) in METHODS {
        let entry = methods
            .iter()
            .find(|entry| entry["method"] == name)
            .unwrap();
        assert_eq!(entry["blocks_thread"], blocks_thread);
        assert!(!entry["description"].as_str().unwrap().is_empty());
    }
}

#[tokio::test]
async fn every_method_accepts_zero_duration_with_the_same_response_schema() {
    let app = app();
    for (method, _) in METHODS {
        let response = app
            .clone()
            .oneshot(post("/sleep", json!({"duration_ms": 0, "method": method})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{method}");
        assert_result(&json_body(response).await, "sleep", method, 0, 200);
    }
}

#[tokio::test]
async fn named_routes_select_their_method_and_reject_body_overrides() {
    let app = app();
    for (path, method) in [
        ("/sleep/tokio", "tokio_sleep"),
        ("/sleep/tokio-spawn", "tokio_spawn"),
        ("/sleep/thread", "thread_sleep"),
        ("/sleep/park", "thread_park"),
    ] {
        let response = app
            .clone()
            .oneshot(post(path, json!({"duration_ms": 0, "status_code": 503})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_result(&json_body(response).await, "sleep", method, 0, 503);

        let response = app
            .clone()
            .oneshot(post(
                path,
                json!({"duration_ms": 0, "method": "tokio_sleep"}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_api_error(response).await;
    }
}

#[tokio::test]
async fn scenario_discovery_lists_the_supported_presets() {
    let response = app().oneshot(get("/scenarios")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let scenarios = body["scenarios"].as_array().unwrap();
    assert_eq!(scenarios.len(), SCENARIOS.len());
    for (name, duration_ms, status_code) in SCENARIOS {
        let entry = scenarios
            .iter()
            .find(|entry| entry["scenario"] == name)
            .unwrap();
        assert_eq!(entry["duration_ms"], duration_ms);
        assert_eq!(entry["status_code"], status_code);
        assert!(!entry["description"].as_str().unwrap().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn async_named_routes_apply_each_scenarios_delay_and_status() {
    let app = app();
    for (path, method) in [
        ("/sleep/tokio", "tokio_sleep"),
        ("/sleep/tokio-spawn", "tokio_spawn"),
    ] {
        for (scenario, duration_ms, status_code) in SCENARIOS {
            let mut request = Box::pin(
                app.clone()
                    .oneshot(post(path, json!({"scenario": scenario}))),
            );
            assert!(
                poll_once(request.as_mut()).is_pending(),
                "{path}: {scenario}"
            );
            // A spawned timer must initialize before virtual time advances.
            tokio::task::yield_now().await;
            tokio::time::advance(Duration::from_millis(duration_ms - 1)).await;
            assert!(
                poll_once(request.as_mut()).is_pending(),
                "{path}: {scenario}"
            );
            tokio::time::advance(Duration::from_millis(1)).await;
            let response = request.await.unwrap();
            assert_eq!(response.status().as_u16(), status_code);
            let body = json_body(response).await;
            assert_scenario_result(
                &body,
                "sleep",
                method,
                duration_ms,
                status_code,
                Some(scenario),
            );
            assert!(body["elapsed_ms"].as_f64().unwrap() >= duration_ms as f64);
        }
    }
}

#[tokio::test]
async fn every_post_route_supports_scenarios_and_a_zero_duration_override() {
    let app = app();
    for (path, operation, method, selectable_method) in [
        ("/sleep/tokio", "sleep", "tokio_sleep", false),
        ("/sleep/tokio-spawn", "sleep", "tokio_spawn", false),
        ("/sleep/thread", "sleep", "thread_sleep", false),
        ("/sleep/park", "sleep", "thread_park", false),
        ("/sleep", "sleep", "tokio_spawn", true),
        ("/simulate/email", "email", "thread_sleep", true),
        ("/simulate/sms", "sms", "thread_park", true),
    ] {
        for (scenario, _, status_code) in SCENARIOS {
            let mut payload = json!({"scenario": scenario, "duration_ms": 0});
            if selectable_method {
                payload["method"] = json!(method);
            }
            let response = app.clone().oneshot(post(path, payload)).await.unwrap();
            assert_eq!(
                response.status().as_u16(),
                status_code,
                "{path}: {scenario}"
            );
            if matches!(status_code, 429 | 503) {
                assert_eq!(response.headers()[header::RETRY_AFTER], "1");
            } else {
                assert!(!response.headers().contains_key(header::RETRY_AFTER));
            }
            assert_scenario_result(
                &json_body(response).await,
                operation,
                method,
                0,
                status_code,
                Some(scenario),
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn explicit_duration_can_shorten_or_extend_a_preset() {
    for duration_ms in [7, 75] {
        let mut request = Box::pin(app().oneshot(post(
            "/sleep/tokio",
            json!({"scenario": "fast_success", "duration_ms": duration_ms}),
        )));
        assert!(poll_once(request.as_mut()).is_pending());
        tokio::time::advance(Duration::from_millis(duration_ms - 1)).await;
        assert!(poll_once(request.as_mut()).is_pending());
        tokio::time::advance(Duration::from_millis(1)).await;
        let response = request.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_scenario_result(
            &body,
            "sleep",
            "tokio_sleep",
            duration_ms,
            200,
            Some("fast_success"),
        );
        assert!(body["elapsed_ms"].as_f64().unwrap() >= duration_ms as f64);
    }
}

#[tokio::test]
async fn unknown_scenarios_and_any_explicit_status_with_a_scenario_are_rejected() {
    let app = app();
    for path in [
        "/sleep/tokio",
        "/sleep/tokio-spawn",
        "/sleep/thread",
        "/sleep/park",
        "/sleep",
        "/simulate/email",
        "/simulate/sms",
    ] {
        for payload in [
            json!({"scenario": "unknown", "duration_ms": 0}),
            json!({"scenario": "fast_success", "duration_ms": 0, "status_code": 200}),
            json!({"scenario": "fast_success", "duration_ms": 0, "status_code": 503}),
        ] {
            let response = app.clone().oneshot(post(path, payload)).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{path}"
            );
            let body = assert_api_error(response).await;
            assert_eq!(body["error"]["retryable"], false);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn duration_limit_applies_after_resolving_a_preset_and_its_override() {
    let app = router(Config {
        max_duration_ms: 100,
        ..Config::default()
    })
    .unwrap();
    for payload in [
        json!({"scenario": "slow_provider"}),
        json!({"scenario": "fast_success", "duration_ms": 101}),
    ] {
        let response = app
            .clone()
            .oneshot(post("/sleep/tokio", payload))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_api_error(response).await;
    }
    for duration_ms in [0, 100] {
        let response = app
            .clone()
            .oneshot(post(
                "/sleep/tokio",
                json!({"scenario": "slow_provider", "duration_ms": duration_ms}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_scenario_result(
            &json_body(response).await,
            "sleep",
            "tokio_sleep",
            duration_ms,
            200,
            Some("slow_provider"),
        );
    }
}

#[tokio::test]
async fn simulated_failures_have_status_specific_errors_and_retry_after_headers() {
    for (status, code, retryable) in [
        (400, "invalid_request", false),
        (429, "rate_limited", true),
        (503, "service_unavailable", true),
        (504, "upstream_timeout", true),
    ] {
        let response = app()
            .oneshot(post(
                "/sleep/tokio",
                json!({"duration_ms": 0, "status_code": status}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        if matches!(status, 429 | 503) {
            assert_eq!(response.headers()[header::RETRY_AFTER], "1");
        } else {
            assert!(!response.headers().contains_key(header::RETRY_AFTER));
        }
        let body = json_body(response).await;
        assert_result(&body, "sleep", "tokio_sleep", 0, status);
        assert_eq!(body["error"]["code"], code);
        assert_eq!(body["error"]["retryable"], retryable);
    }
    for status in [200, 302, 500, 599] {
        let response = app()
            .oneshot(post(
                "/sleep/tokio",
                json!({"duration_ms": 0, "status_code": status}),
            ))
            .await
            .unwrap();
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
        assert_result(
            &json_body(response).await,
            "sleep",
            "tokio_sleep",
            0,
            status,
        );
    }
}

#[tokio::test]
async fn simulated_auth_and_method_failures_include_provider_headers() {
    for (status, expected_code, expected_header, expected_value) in [
        (
            401,
            "unauthorized",
            header::WWW_AUTHENTICATE,
            "Bearer realm=\"sleepapi\"",
        ),
        (405, "method_not_allowed", header::ALLOW, "GET"),
    ] {
        let response = app()
            .oneshot(post(
                "/sleep/tokio",
                json!({"duration_ms": 0, "status_code": status}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers()[expected_header], expected_value);
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
        let body = json_body(response).await;
        assert_result(&body, "sleep", "tokio_sleep", 0, status);
        assert_eq!(body["error"]["code"], expected_code);
        assert_eq!(body["error"]["retryable"], false);
    }
    let response = app()
        .oneshot(post("/sleep/tokio", json!({"duration_ms": 0})))
        .await
        .unwrap();
    assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
    assert!(!response.headers().contains_key(header::ALLOW));
}

#[tokio::test(start_paused = true)]
async fn empty_object_uses_the_documented_default_delay() {
    let mut response = Box::pin(app().oneshot(post("/sleep", json!({}))));
    assert!(poll_once(response.as_mut()).is_pending());
    tokio::time::advance(Duration::from_millis(399)).await;
    assert!(poll_once(response.as_mut()).is_pending());
    tokio::time::advance(Duration::from_millis(1)).await;
    let response = response.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_result(&body, "sleep", "tokio_sleep", 400, 200);
    assert!(body["elapsed_ms"].as_f64().unwrap() >= 400.0);
}

#[tokio::test(start_paused = true)]
async fn async_methods_wait_concurrently_on_virtual_time() {
    let app = app();
    let mut requests = Vec::new();
    for method in ["tokio_sleep", "tokio_spawn"] {
        let mut request = Box::pin(app.clone().oneshot(post(
            "/sleep",
            json!({"duration_ms": 100, "method": method}),
        )));
        assert!(poll_once(request.as_mut()).is_pending());
        requests.push((method, request));
    }
    // Allow the separately spawned timer to be initialized before advancing time.
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(99)).await;
    for (_, request) in &mut requests {
        assert!(poll_once(request.as_mut()).is_pending());
    }
    tokio::time::advance(Duration::from_millis(1)).await;
    for (method, request) in requests {
        let response = request.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_result(&body, "sleep", method, 100, 200);
        assert!(body["elapsed_ms"].as_f64().unwrap() >= 100.0);
    }
}

#[tokio::test]
async fn every_method_waits_at_least_the_requested_wall_clock_duration() {
    let app = app();
    for (method, _) in METHODS {
        let started = Instant::now();
        let response = app
            .clone()
            .oneshot(post("/sleep", json!({"duration_ms": 15, "method": method})))
            .await
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(15), "{method}");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_result(&body, "sleep", method, 15, 200);
        assert!(body["elapsed_ms"].as_f64().unwrap() >= 15.0, "{body}");
    }
}

#[tokio::test(start_paused = true)]
async fn email_and_sms_return_simulated_results_with_requested_http_status() {
    for (operation, status) in [("email", 202), ("sms", 503)] {
        let response = app()
            .oneshot(post(
                &format!("/simulate/{operation}"),
                json!({"duration_ms": 20, "status_code": status}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        let body = json_body(response).await;
        assert_result(&body, operation, "tokio_sleep", 20, status);
        assert!(body["elapsed_ms"].as_f64().unwrap() >= 20.0);
    }
}

#[tokio::test]
async fn supported_status_boundaries_and_redirects_preserve_json() {
    for status in [200, 299, 302, 400, 500, 599] {
        let response = app()
            .oneshot(post(
                "/sleep",
                json!({"duration_ms": 0, "status_code": status}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_result(
            &json_body(response).await,
            "sleep",
            "tokio_sleep",
            0,
            status,
        );
    }
}

#[tokio::test]
async fn duration_and_unsupported_statuses_are_rejected() {
    let app = router(Config {
        max_duration_ms: 20,
        ..Config::default()
    })
    .unwrap();
    for payload in [
        json!({"duration_ms": 21}),
        json!({"duration_ms": u64::MAX}),
        json!({"duration_ms": -1}),
        json!({"duration_ms": 0.5}),
        json!({"duration_ms": "10"}),
    ] {
        let response = app.clone().oneshot(post("/sleep", payload)).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_api_error(response).await;
    }
    for status in [
        0, 100, 199, 204, 205, 206, 226, 304, 407, 416, 426, 600, 999,
    ] {
        let response = app
            .clone()
            .oneshot(post(
                "/sleep",
                json!({"duration_ms": 0, "status_code": status}),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{status}"
        );
        let body = assert_api_error(response).await;
        assert_eq!(body["error"]["code"], "validation_failed");
        assert_eq!(body["error"]["retryable"], false);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("status_code")
        );
    }
}

#[tokio::test]
async fn malformed_json_invalid_schema_and_missing_content_type_return_json_errors() {
    let app = app();
    for (body, has_content_type, expected) in [
        ("{", true, StatusCode::BAD_REQUEST),
        ("", true, StatusCode::BAD_REQUEST),
        ("{}", false, StatusCode::UNSUPPORTED_MEDIA_TYPE),
        ("null", true, StatusCode::UNPROCESSABLE_ENTITY),
        (
            r#"{"method":"unknown"}"#,
            true,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (r#"{"duration":1}"#, true, StatusCode::UNPROCESSABLE_ENTITY),
        (
            r#"{"status_code":70000}"#,
            true,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(raw_post("/sleep", body.to_owned(), has_content_type))
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "body: {body}");
        assert_api_error(response).await;
    }
}

#[tokio::test]
async fn body_limit_accepts_eight_kib_and_rejects_one_more_byte() {
    let app = app();
    let mut body = r#"{"duration_ms":0}"#.to_owned();
    body.extend(std::iter::repeat_n(' ', 8 * 1024 - body.len()));
    let response = app
        .clone()
        .oneshot(raw_post("/sleep", body.clone(), true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body.push(' ');
    let response = app.oneshot(raw_post("/sleep", body, true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_api_error(response).await;
}

#[tokio::test]
async fn unknown_simulation_and_route_return_json_not_found_errors() {
    let app = app();
    for request in [
        post("/simulate/push", json!({"duration_ms": 0})),
        get("/missing"),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_api_error(response).await;
    }
}

#[tokio::test]
async fn an_unsupported_http_method_returns_a_structured_api_error() {
    let response = app().oneshot(get("/sleep/tokio")).await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "POST");
    assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
    let body = assert_api_error(response).await;
    assert_eq!(body["error"]["code"], "method_not_allowed");
    assert_eq!(body["error"]["retryable"], false);
}

#[tokio::test]
async fn invalid_utf8_in_a_simulation_path_returns_a_structured_api_error() {
    let response = app()
        .oneshot(post("/simulate/%FF", json!({"duration_ms": 0})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = assert_api_error(response).await;
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(body["error"]["retryable"], false);
}

#[tokio::test(start_paused = true)]
async fn overload_is_rejected_without_waiting_and_capacity_recovers() {
    let app = router(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .unwrap();
    let simulated = app
        .clone()
        .oneshot(post(
            "/sleep/tokio",
            json!({"scenario": "rate_limited", "duration_ms": 0}),
        ))
        .await
        .unwrap();
    assert_eq!(simulated.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(simulated.headers()[header::RETRY_AFTER], "1");
    let simulated = json_body(simulated).await;
    assert_scenario_result(
        &simulated,
        "sleep",
        "tokio_sleep",
        0,
        429,
        Some("rate_limited"),
    );
    assert_eq!(simulated["error"]["code"], "rate_limited");
    assert_eq!(simulated["error"]["retryable"], true);

    let mut first = Box::pin(
        app.clone()
            .oneshot(post("/sleep", json!({"duration_ms": 100}))),
    );
    assert!(poll_once(first.as_mut()).is_pending());

    // Simulation endpoints share the same capacity limit as /sleep.
    let mut overloaded = Box::pin(
        app.clone()
            .oneshot(post("/simulate/email", json!({"duration_ms": 0}))),
    );
    let Poll::Ready(response) = poll_once(overloaded.as_mut()) else {
        panic!("an overloaded request was queued instead of rejected immediately");
    };
    let response = response.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
    let body = assert_api_error(response).await;
    assert_eq!(body["error"]["code"], "capacity_exceeded");
    assert_eq!(body["error"]["retryable"], true);
    assert_eq!(
        app.clone().oneshot(get("/health")).await.unwrap().status(),
        StatusCode::OK
    );

    assert_eq!(first.await.unwrap().status(), StatusCode::OK);
    let response = app
        .oneshot(post("/simulate/sms", json!({"duration_ms": 0})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn dropping_async_requests_releases_capacity() {
    for method in ["tokio_sleep", "tokio_spawn"] {
        let app = router(Config {
            max_in_flight: 1,
            ..Config::default()
        })
        .unwrap();
        let mut pending = Box::pin(app.clone().oneshot(post(
            "/sleep",
            json!({"duration_ms": 30_000, "method": method}),
        )));
        assert!(poll_once(pending.as_mut()).is_pending());
        tokio::task::yield_now().await;
        drop(pending);
        // Aborting a child task schedules its cleanup rather than doing it inline.
        tokio::task::yield_now().await;
        let response = app
            .oneshot(post("/sleep", json!({"duration_ms": 0})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{method}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn waiting_methods_leave_health_responsive_on_a_single_runtime_thread() {
    for (method, _) in METHODS {
        let app = router(Config {
            max_in_flight: 1,
            ..Config::default()
        })
        .unwrap();
        let mut slow = Box::pin(
            app.clone()
                .oneshot(post("/sleep", json!({"duration_ms": 25, "method": method}))),
        );
        // A direct blocking sleep in the handler would finish during this poll.
        assert!(poll_once(slow.as_mut()).is_pending(), "{method}");
        let mut health = Box::pin(app.oneshot(get("/health")));
        let Poll::Ready(response) = poll_once(health.as_mut()) else {
            panic!("health did not respond while {method} was pending");
        };
        assert_eq!(response.unwrap().status(), StatusCode::OK);
        assert_eq!(slow.await.unwrap().status(), StatusCode::OK);
    }
}
