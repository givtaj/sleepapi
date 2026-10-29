use std::{io, net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, Version, header},
    response::Response,
};
use serde_json::{Value, json};
use sleepapi::{
    Config, router,
    transport::{FaultConnection, FaultListener},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::timeout,
};
use tower::ServiceExt;

struct Server {
    address: SocketAddr,
    task: JoinHandle<io::Result<()>>,
}

impl Server {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = router(Config {
            max_in_flight: 1,
            ..Config::default()
        })
        .unwrap();
        let task = tokio::spawn(async move {
            axum::serve(
                FaultListener::new(listener),
                app.into_make_service_with_connect_info::<FaultConnection>(),
            )
            .await
        });
        Self { address, task }
    }

    async fn exchange(&self, request: Vec<u8>) -> Vec<u8> {
        timeout(Duration::from_secs(5), async {
            let mut stream = TcpStream::connect(self.address).await.unwrap();
            stream.write_all(&request).await.unwrap();
            let mut response = Vec::new();
            match stream.read_to_end(&mut response).await {
                Ok(_) => {}
                // Either a clean EOF or reset denotes the intended unusable
                // connection; all bytes preceding either must still be exact.
                Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                Err(error) => panic!("read failed: {error}"),
            }
            response
        })
        .await
        .expect("HTTP exchange did not terminate")
    }

    async fn json(&self, method: &str, path: &str, payload: Option<Value>) -> (u16, Value) {
        let response = self
            .exchange(request(method, path, payload, "HTTP/1.1", false))
            .await;
        let (head, body) = split(&response);
        assert_eq!(content_length(head), body.len());
        (status(head), serde_json::from_slice(body).unwrap())
    }

    async fn create(&self, sequence: Value) -> String {
        let (status, created) = self
            .json("POST", "/runs", Some(json!({"sequence": sequence})))
            .await;
        assert_eq!(status, 201, "{created}");
        created["run_id"].as_str().unwrap().to_owned()
    }

    async fn attempt(&self, id: &str, generation: u64) -> Vec<u8> {
        self.exchange(request(
            "POST",
            &format!("/runs/{id}/attempts"),
            Some(json!({"generation":generation})),
            "HTTP/1.1",
            false,
        ))
        .await
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn request(
    method: &str,
    path: &str,
    payload: Option<Value>,
    version: &str,
    keep_alive: bool,
) -> Vec<u8> {
    let body = payload.map(|value| value.to_string()).unwrap_or_default();
    format!(
        "{method} {path} {version}\r\nHost: localhost\r\nConnection: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        if keep_alive { "keep-alive" } else { "close" },
        body.len(),
    )
    .into_bytes()
}

fn split(response: &[u8]) -> (&str, &[u8]) {
    let boundary = response
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .expect("missing response headers")
        + 4;
    (
        std::str::from_utf8(&response[..boundary]).unwrap(),
        &response[boundary..],
    )
}

fn status(head: &str) -> u16 {
    head.split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .expect("response must declare Content-Length")
}

async fn read_complete(stream: &mut TcpStream) -> Vec<u8> {
    timeout(Duration::from_secs(5), async {
        let mut response = Vec::new();
        while !response.ends_with(b"\r\n\r\n") {
            response.push(stream.read_u8().await.unwrap());
            assert!(response.len() < 16 * 1024);
        }
        let body_length = content_length(std::str::from_utf8(&response).unwrap());
        let mut body = vec![0; body_length];
        stream.read_exact(&mut body).await.unwrap();
        response.extend(body);
        response
    })
    .await
    .expect("complete HTTP response did not arrive")
}

#[tokio::test]
async fn tcp_fault_sequence_recovers_and_reset_repeats_exact_wire_failures() {
    let server = Server::start().await;
    let fixture = "café 💤 ok";
    let id = server
        .create(json!([
            {"status_code":503,"duration_ms":0,"fault":{"type":"connection_drop"}},
            {"status_code":502,"duration_ms":0,"fixture":{"body":fixture,"content_type":"text/plain"},
             "fault":{"type":"truncated_response","after_bytes":4}},
            {"status_code":200,"duration_ms":0,"fixture":{"body":"recovered","content_type":"text/plain"}}
        ]))
        .await;

    for generation in [1, 2] {
        assert!(
            server.attempt(&id, generation).await.is_empty(),
            "drop must emit no HTTP status or headers"
        );
        let truncated = server.attempt(&id, generation).await;
        let (head, prefix) = split(&truncated);
        assert_eq!(status(head), 502);
        assert_eq!(content_length(head), fixture.len());
        assert!(!head.to_ascii_lowercase().contains("transfer-encoding:"));
        assert_eq!(prefix, &fixture.as_bytes()[..4]);
        assert!(
            std::str::from_utf8(prefix).is_err(),
            "cutoff counts bytes, even inside UTF-8"
        );
        assert!(
            prefix.len() < content_length(head),
            "response is necessarily incomplete"
        );

        let normal = server.attempt(&id, generation).await;
        let (head, body) = split(&normal);
        assert_eq!(status(head), 200);
        assert_eq!(body, b"recovered");
        assert_eq!(content_length(head), body.len());
        let (status, receipt) = server
            .json("GET", &format!("/runs/{id}/receipts"), None)
            .await;
        assert_eq!(status, 200);
        assert_eq!(receipt["receipts"][0]["outcome"], "fault_injected");
        assert_eq!(receipt["receipts"][0]["injection_stage"], "before_headers");
        assert_eq!(receipt["receipts"][0]["injected_body_bytes"], 0);
        assert_eq!(receipt["receipts"][1]["outcome"], "fault_injected");
        assert_eq!(
            receipt["receipts"][1]["injection_stage"],
            "after_body_prefix"
        );
        assert_eq!(receipt["receipts"][1]["injected_body_bytes"], 4);
        assert_eq!(receipt["receipts"][2]["outcome"], "completed");
        assert!(receipt["receipts"][2].get("injection_stage").is_none());
        assert_eq!(server.json("GET", "/health", None).await.0, 200);
        if generation == 1 {
            let (status, reset) = server
                .json(
                    "POST",
                    &format!("/runs/{id}/reset"),
                    Some(json!({"generation":1})),
                )
                .await;
            assert_eq!(status, 200);
            assert_eq!(reset["generation"], 2);
        }
    }
}

#[tokio::test]
async fn tcp_faults_work_with_all_wait_methods_and_release_shared_capacity() {
    let server = Server::start().await;
    for method in ["tokio_sleep", "tokio_spawn", "thread_sleep", "thread_park"] {
        let id = server.create(json!([
            {"status_code":200,"duration_ms":1,"method":method,"fault":{"type":"connection_drop"}},
            {"status_code":200,"duration_ms":1,"method":method,"fixture":{"body":"abcdef"},
             "fault":{"type":"truncated_response","after_bytes":3}},
            {"status_code":200,"duration_ms":0}
        ])).await;
        assert!(server.attempt(&id, 1).await.is_empty());
        let response = server.attempt(&id, 1).await;
        let (head, prefix) = split(&response);
        assert_eq!(content_length(head), 6);
        assert_eq!(prefix, b"abc");
        let final_response = server.attempt(&id, 1).await;
        assert_eq!(
            status(split(&final_response).0),
            200,
            "capacity leaked for {method}"
        );
    }
}

#[tokio::test]
async fn http_10_fault_rejection_does_not_consume_step() {
    let server = Server::start().await;
    let id = server
        .create(json!([
            {"status_code":200,"duration_ms":0,"fault":{"type":"connection_drop"}}
        ]))
        .await;
    let rejected = server
        .exchange(request(
            "POST",
            &format!("/runs/{id}/attempts"),
            Some(json!({"generation":1})),
            "HTTP/1.0",
            false,
        ))
        .await;
    let (head, body) = split(&rejected);
    assert!((400..600).contains(&status(head)));
    assert_eq!(
        serde_json::from_slice::<Value>(body).unwrap()["simulated"],
        false
    );
    let (_, summary) = server.json("GET", &format!("/runs/{id}"), None).await;
    assert_eq!(summary["attempts_started"], 0);
    assert!(server.attempt(&id, 1).await.is_empty());
}

#[tokio::test]
async fn normal_keepalive_and_prior_pipelined_response_survive_fault_handling() {
    let server = Server::start().await;
    let mut stream = TcpStream::connect(server.address).await.unwrap();
    for _ in 0..2 {
        stream
            .write_all(&request("GET", "/health", None, "HTTP/1.1", true))
            .await
            .unwrap();
        let response = read_complete(&mut stream).await;
        assert_eq!(status(split(&response).0), 200);
        assert_eq!(
            serde_json::from_slice::<Value>(split(&response).1).unwrap()["status"],
            "ok"
        );
    }
    let id = server
        .create(json!([
            {"status_code":200,"duration_ms":0,"fault":{"type":"connection_drop"}}
        ]))
        .await;
    let mut pipelined = request("GET", "/health", None, "HTTP/1.1", true);
    pipelined.extend(request(
        "POST",
        &format!("/runs/{id}/attempts"),
        Some(json!({"generation":1})),
        "HTTP/1.1",
        true,
    ));
    stream.write_all(&pipelined).await.unwrap();
    let normal = read_complete(&mut stream).await;
    assert_eq!(status(split(&normal).0), 200);
    let mut after = Vec::new();
    let ended = timeout(Duration::from_secs(5), stream.read_to_end(&mut after))
        .await
        .unwrap();
    assert!(ended.is_ok() || ended.unwrap_err().kind() == io::ErrorKind::ConnectionReset);
    assert!(after.is_empty(), "fault must not emit a second status line");
}

#[tokio::test]
async fn pipelined_truncation_flushes_its_own_prefix_after_a_complete_response() {
    let server = Server::start().await;
    let normal_body = "x".repeat(3072);
    let truncated_body = "é".repeat(1024);
    let id = server
        .create(json!([
            {"status_code":200,"duration_ms":0,"fixture":{"body":normal_body}},
            {"status_code":503,"duration_ms":0,"fixture":{"body":truncated_body},
             "fault":{"type":"truncated_response","after_bytes":777}}
        ]))
        .await;
    let mut stream = TcpStream::connect(server.address).await.unwrap();
    let one_request = request(
        "POST",
        &format!("/runs/{id}/attempts"),
        Some(json!({"generation":1})),
        "HTTP/1.1",
        true,
    );
    let mut pipeline = one_request.clone();
    pipeline.extend(one_request);
    stream.write_all(&pipeline).await.unwrap();
    let normal = read_complete(&mut stream).await;
    assert_eq!(status(split(&normal).0), 200);
    assert_eq!(split(&normal).1, normal_body.as_bytes());
    let mut truncated = Vec::new();
    let ended = timeout(Duration::from_secs(5), stream.read_to_end(&mut truncated))
        .await
        .unwrap();
    assert!(ended.is_ok() || ended.unwrap_err().kind() == io::ErrorKind::ConnectionReset);
    let (head, prefix) = split(&truncated);
    assert_eq!(status(head), 503);
    assert_eq!(content_length(head), truncated_body.len());
    assert_eq!(prefix, &truncated_body.as_bytes()[..777]);
}

fn router_request(method: &str, path: &str, payload: Option<Value>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            payload.map(|v| v.to_string()).unwrap_or_default(),
        ))
        .unwrap()
}

async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

async fn direct_json(app: &Router, method: &str, path: &str, payload: Option<Value>) -> Value {
    json_body(
        app.clone()
            .oneshot(router_request(method, path, payload))
            .await
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn invalid_fault_configuration_is_rejected_without_allocating_any_run() {
    let app = router(Config::default()).unwrap();
    let mut invalid = vec![
        json!({"fault":{"type":"truncated_response","after_bytes":1}}),
        json!({"fixture":{"body":""},"fault":{"type":"truncated_response","after_bytes":1}}),
        json!({"fixture":{"body":"a"},"fault":{"type":"truncated_response","after_bytes":0}}),
        json!({"fixture":{"body":"a"},"fault":{"type":"truncated_response","after_bytes":1}}),
        json!({"fixture":{"body":"abc"},"fault":{"type":"truncated_response","after_bytes":0}}),
        json!({"fixture":{"body":"abc"},"fault":{"type":"truncated_response","after_bytes":3}}),
        json!({"fixture":{"body":"abc"},"fault":{"type":"truncated_response","after_bytes":4}}),
        json!({"fixture":{"body":"abc"},"fault":{"type":"truncated_response","after_bytes":-1}}),
        json!({"fixture":{"body":"abc"},"fault":{"type":"truncated_response"}}),
        json!({"fixture":{"body":"abc"},"fault":{"type":"truncated_response","after_bytes":1,"extra":true}}),
        json!({"fault":{"type":"connection_drop","after_bytes":0}}),
        json!({"fault":{"type":"connection_drop","extra":true}}),
        json!({"fault":{"type":"unknown_fault"}}),
        json!({"fault":{}}),
    ];
    for step in &mut invalid {
        step["status_code"] = json!(200);
        step["duration_ms"] = json!(0);
        let response = app
            .clone()
            .oneshot(router_request(
                "POST",
                "/runs",
                // Also prove that earlier valid steps do not partially allocate.
                Some(json!({"sequence":[{"status_code":200,"duration_ms":0}, step]})),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(json_body(response).await["simulated"], false);
        let discovery = direct_json(&app, "GET", "/runs", None).await;
        assert_eq!(discovery["active_runs"], 0);
    }
}

#[tokio::test]
async fn missing_transport_and_http2_attempts_leave_sequence_and_capacity_untouched() {
    let app = router(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .unwrap();
    let created = direct_json(
        &app,
        "POST",
        "/runs",
        Some(json!({"sequence":[
            {"status_code":200,"duration_ms":0,"fault":{"type":"connection_drop"}}
        ]})),
    )
    .await;
    let id = created["run_id"].as_str().unwrap();
    for (version, connected) in [(Version::HTTP_11, false), (Version::HTTP_2, true)] {
        let mut attempt = router_request(
            "POST",
            &format!("/runs/{id}/attempts"),
            Some(json!({"generation":1})),
        );
        *attempt.version_mut() = version;
        if connected {
            attempt
                .extensions_mut()
                .insert(ConnectInfo(FaultConnection::default()));
        }
        let rejected = app.clone().oneshot(attempt).await.unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(json_body(rejected).await["simulated"], false);
        let summary = direct_json(&app, "GET", &format!("/runs/{id}"), None).await;
        assert_eq!(summary["attempts_started"], 0);
        assert_eq!(summary["generation_attempts"], 0);
        assert_eq!(summary["retained_receipts"], 0);
        let sleep = app
            .clone()
            .oneshot(router_request(
                "POST",
                "/sleep",
                Some(json!({"duration_ms":0})),
            ))
            .await
            .unwrap();
        assert_eq!(sleep.status(), StatusCode::OK, "rejection leaked capacity");
    }
}
