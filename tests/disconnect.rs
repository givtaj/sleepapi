//! Wire-level cancellation checks for ordinary recovery responses. Injected
//! connection-drop/truncation faults have separate coverage in transport.rs.
//! These specific socket closures exercise cancellation; they do not promise
//! that every client disconnect immediately cancels an HTTP handler.
#![cfg(unix)]

use std::{
    io,
    net::{Shutdown, SocketAddr},
    os::fd::AsRawFd,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use sleepapi::{
    Config, router,
    transport::{FaultConnection, FaultListener},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};

const OBSERVATION_LIMIT: Duration = Duration::from_secs(5);
const BLOCKING_WAIT_MS: u64 = 2_000;
const METHODS: [&str; 4] = ["tokio_sleep", "tokio_spawn", "thread_sleep", "thread_park"];

#[derive(Clone, Copy, Debug)]
enum Disconnect {
    Fin,
    Reset,
}

impl Disconnect {
    fn close(self, stream: TcpStream) {
        let stream = stream.into_std().unwrap();
        match self {
            Self::Fin => {
                // Shut down both directions and close the socket. A write-only
                // half-close still permits reading a response and is not the
                // abandoned-client case exercised here.
                stream.shutdown(Shutdown::Both).unwrap();
            }
            Self::Reset => {
                let linger = libc::linger {
                    l_onoff: 1,
                    l_linger: 0,
                };
                // SAFETY: stream owns this live descriptor and linger is valid
                // for the supplied size throughout the synchronous call.
                let result = unsafe {
                    libc::setsockopt(
                        stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_LINGER,
                        (&linger as *const libc::linger).cast(),
                        std::mem::size_of_val(&linger) as libc::socklen_t,
                    )
                };
                assert_eq!(result, 0, "set SO_LINGER: {}", io::Error::last_os_error());
            }
        }
        drop(stream);
    }
}

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

    async fn begin(&self, method: &str, path: &str, payload: Option<Value>) -> TcpStream {
        timeout(OBSERVATION_LIMIT, async {
            let body = payload.map(|value| value.to_string()).unwrap_or_default();
            let request = format!(
                "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let mut stream = TcpStream::connect(self.address).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            stream
        })
        .await
        .expect("request write timed out")
    }

    async fn json(&self, method: &str, path: &str, payload: Option<Value>) -> (u16, Value) {
        let mut stream = self.begin(method, path, payload).await;
        let mut response = Vec::new();
        timeout(OBSERVATION_LIMIT, stream.read_to_end(&mut response))
            .await
            .expect("response read timed out")
            .unwrap();
        let boundary = response
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap()
            + 4;
        let head = std::str::from_utf8(&response[..boundary]).unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        let body = &response[boundary..];
        let value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(body).unwrap()
        };
        (status, value)
    }

    async fn receipts(&self, id: &str) -> Value {
        let (status, value) = self
            .json("GET", &format!("/runs/{id}/receipts"), None)
            .await;
        assert_eq!(status, 200, "{value}");
        value
    }

    async fn pending(&self, method: &str) -> (String, TcpStream, Instant) {
        let blocking = method.starts_with("thread_");
        // Warm the blocking pool before the request. The API exposes admission,
        // not a worker-start event; the allowance below lets this prewarmed
        // worker start before the socket is closed.
        if blocking {
            tokio::task::spawn_blocking(|| ()).await.unwrap();
        }
        let delay = if blocking { BLOCKING_WAIT_MS } else { 30_000 };
        let (status, created) = self
            .json(
                "POST",
                "/runs",
                Some(json!({"sequence":[
                    {"status_code":503,"duration_ms":delay,"method":method},
                    {"status_code":200,"duration_ms":0,"method":method}
                ]})),
            )
            .await;
        assert_eq!(status, 201, "{created}");
        let id = created["run_id"].as_str().unwrap().to_owned();
        let issued = Instant::now();
        let stream = self
            .begin(
                "POST",
                &format!("/runs/{id}/attempts"),
                Some(json!({"generation":1})),
            )
            .await;
        timeout(OBSERVATION_LIMIT, async {
            loop {
                let records = self.receipts(&id).await;
                if let Some(receipt) = records["receipts"].as_array().unwrap().first() {
                    assert_eq!(receipt["outcome"], "pending", "{method}: {records}");
                    assert_eq!(receipt["attempt"], 1);
                    break;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("attempt was not admitted");
        if blocking {
            sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            self.receipts(&id).await["receipts"][0]["outcome"],
            "pending",
            "the {method} wait must still be active before disconnect"
        );
        assert_eq!(
            self.probe().await,
            429,
            "pending {method} must retain capacity"
        );
        (id, stream, issued)
    }

    async fn probe(&self) -> u16 {
        let (status, value) = self
            .json("POST", "/sleep/tokio", Some(json!({"duration_ms":0})))
            .await;
        if status == 429 {
            assert_eq!(value["error"]["code"], "capacity_exceeded");
        } else {
            assert_eq!(status, 200, "{value}");
        }
        status
    }

    async fn recovered(&self) {
        timeout(OBSERVATION_LIMIT, async {
            while self.probe().await == 429 {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("capacity did not recover");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn cancellation(method: &'static str, disconnect: Disconnect) {
    let server = Server::start().await;
    let (id, stream, issued) = server.pending(method).await;
    disconnect.close(stream);
    timeout(OBSERVATION_LIMIT, async {
        loop {
            let records = server.receipts(&id).await;
            match records["receipts"][0]["outcome"].as_str().unwrap() {
                "pending" => sleep(Duration::from_millis(5)).await,
                "handler_cancelled" => break,
                outcome => panic!("{method}/{disconnect:?}: expected cancellation, got {outcome}"),
            }
        }
    })
    .await
    .expect("disconnect did not cancel the handler");
    if method.starts_with("thread_") {
        assert_eq!(
            server.probe().await,
            429,
            "started {method} must outlive its cancelled handler"
        );
    } else {
        // A cancelled spawned timer releases its permit when abort cleanup runs.
        timeout(Duration::from_secs(2), server.recovered())
            .await
            .expect("async cancellation retained a 30-second wait");
    }
    assert_eq!(server.json("GET", "/health", None).await.0, 200);
    server.recovered().await;
    if method.starts_with("thread_") {
        assert!(
            issued.elapsed() >= Duration::from_millis(BLOCKING_WAIT_MS),
            "{method} released its permit before its configured wait could finish"
        );
    }
    let (status, next) = server
        .json(
            "POST",
            &format!("/runs/{id}/attempts"),
            Some(json!({"generation":1})),
        )
        .await;
    assert_eq!(status, 200, "{method}/{disconnect:?}: {next}");
    assert_eq!(next["attempt"], 2);
    assert_eq!(next["sequence_step"], 2);
    let records = server.receipts(&id).await;
    assert_eq!(records["receipts"].as_array().unwrap().len(), 2);
    assert_eq!(records["receipts"][0]["outcome"], "handler_cancelled");
    assert_eq!(records["receipts"][1]["outcome"], "completed");
    let (_, summary) = server.json("GET", &format!("/runs/{id}"), None).await;
    assert_eq!(summary["attempts_started"], 2);
}

async fn cleared_state(method: &'static str, disconnect: Disconnect, delete: bool) {
    let server = Server::start().await;
    let (id, stream, _) = server.pending(method).await;
    if delete {
        assert_eq!(
            server.json("DELETE", &format!("/runs/{id}"), None).await.0,
            204
        );
    } else {
        let (status, reset) = server
            .json(
                "POST",
                &format!("/runs/{id}/reset"),
                Some(json!({"generation":1})),
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(reset["generation"], 2);
        assert_eq!(reset["attempts_started"], 1);
        assert_eq!(reset["generation_attempts"], 0);
        assert!(
            server.receipts(&id).await["receipts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    // Clear while the old handler is still admitted, then deliver the actual
    // socket disconnect. Both late cancellation and blocking completion follow
    // the reset/deletion and must not recreate its diagnostic state.
    disconnect.close(stream);
    server.recovered().await;
    if delete {
        for suffix in ["", "/receipts"] {
            assert_eq!(
                server
                    .json("GET", &format!("/runs/{id}{suffix}"), None)
                    .await
                    .0,
                404
            );
        }
        assert_eq!(server.json("GET", "/runs", None).await.1["active_runs"], 0);
    } else {
        let records = server.receipts(&id).await;
        assert_eq!(records["generation"], 2);
        assert!(records["receipts"].as_array().unwrap().is_empty());
        let (_, summary) = server.json("GET", &format!("/runs/{id}"), None).await;
        assert_eq!(summary["generation"], 2);
        assert_eq!(summary["attempts_started"], 1);
        assert_eq!(summary["generation_attempts"], 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tcp_disconnect_consumes_the_attempt_and_preserves_wait_capacity_rules() {
    let mut cases = JoinSet::new();
    for method in METHODS {
        for disconnect in [Disconnect::Fin, Disconnect::Reset] {
            cases.spawn(cancellation(method, disconnect));
        }
    }
    while let Some(result) = cases.join_next().await {
        result.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tcp_disconnect_after_reset_cannot_restore_old_generation_receipts() {
    let mut cases = JoinSet::new();
    for method in METHODS {
        for disconnect in [Disconnect::Fin, Disconnect::Reset] {
            cases.spawn(cleared_state(method, disconnect, false));
        }
    }
    while let Some(result) = cases.join_next().await {
        result.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tcp_disconnect_after_delete_cannot_recreate_run_or_receipts() {
    let mut cases = JoinSet::new();
    for method in METHODS {
        for disconnect in [Disconnect::Fin, Disconnect::Reset] {
            cases.spawn(cleared_state(method, disconnect, true));
        }
    }
    while let Some(result) = cases.join_next().await {
        result.unwrap();
    }
}
