#![cfg(unix)]

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    logs: Receiver<String>,
    address: SocketAddr,
}

impl Server {
    fn start(grace_ms: u64) -> Self {
        let mut server = Self::spawn("127.0.0.1:0", grace_ms);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let line = server
                .logs
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("server did not announce its listening address");
            if !line.contains("sleepapi listening") {
                continue;
            }
            let port = line
                .split_once("127.0.0.1:")
                .expect("startup log should contain the loopback address")
                .1
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u16>()
                .unwrap();
            server.address.set_port(port);
            return server;
        }
    }

    fn spawn(address: &str, grace_ms: u64) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sleepapi"))
            .env("SLEEPAPI_ADDR", address)
            .env("SLEEPAPI_MAX_DURATION_MS", "10000")
            .env("SLEEPAPI_MAX_IN_FLIGHT", "1")
            .env("SLEEPAPI_SHUTDOWN_GRACE_MS", grace_ms.to_string())
            .env("SLEEPAPI_CORS_ORIGINS", "")
            .env("TOKIO_WORKER_THREADS", "2")
            .env("RUST_LOG", "sleepapi=info")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start sleepapi process");
        let stdout = child.stdout.take().unwrap();
        let (logs_tx, logs) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if logs_tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            logs,
            address: "127.0.0.1:0".parse().unwrap(),
        }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect_timeout(&self.address, Duration::from_secs(2)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
    }

    fn request(&self, method: &str, duration_ms: u64) -> TcpStream {
        let mut stream = self.connect();
        let body = format!(r#"{{"duration_ms":{duration_ms}}}"#);
        write!(
            stream,
            "POST /sleep/{method} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        stream
    }

    fn incomplete_request(&self) -> TcpStream {
        let mut stream = self.connect();
        stream
            .write_all(b"POST /sleep/tokio HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 20\r\nExpect: 100-continue\r\n\r\n")
            .unwrap();
        let mut interim = [0; 25];
        stream.read_exact(&mut interim).unwrap();
        assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
        stream.write_all(b"{").unwrap();
        stream
    }

    fn waiting_request(&self, method: &str, duration_ms: u64) -> TcpStream {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                Instant::now() < deadline,
                "long request never occupied its slot"
            );
            let request = self.request(method, duration_ms);
            loop {
                let probe = response(self.request("tokio", 0));
                if probe.starts_with("HTTP/1.1 429") {
                    return request;
                }
                // The zero-duration probe can win the admission race, causing
                // the long request itself to get 429. Consume that response and
                // retry, rather than relying on a guessed thread-start delay.
                request.set_nonblocking(true).unwrap();
                let ready = request.peek(&mut [0; 1]);
                request.set_nonblocking(false).unwrap();
                match ready {
                    Ok(0) => panic!("long request closed before a response"),
                    Ok(_) => {
                        let completed = response(request);
                        assert!(
                            completed.starts_with("HTTP/1.1 429")
                                || completed.starts_with("HTTP/1.1 200"),
                            "unexpected response: {completed}"
                        );
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("inspect long request: {error}"),
                }
                assert!(
                    Instant::now() < deadline,
                    "long request never occupied its slot"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn signal(&self, name: &str) {
        let status = Command::new("kill")
            .arg(format!("-{name}"))
            .arg(self.child.id().to_string())
            .status()
            .unwrap();
        assert!(status.success(), "could not signal test child");
    }

    fn wait_for_shutdown_log(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let line = self
                .logs
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("server did not start shutting down");
            if line.contains("shutdown requested") {
                return;
            }
        }
    }

    fn assert_port_closed(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while TcpStream::connect_timeout(&self.address, Duration::from_millis(50)).is_ok() {
            assert!(
                Instant::now() < deadline,
                "server kept accepting connections during shutdown"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "server failed to exit within {timeout:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn response(mut stream: TcpStream) -> String {
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn sigterm_closes_listener_and_drains_an_active_request() {
    let mut server = Server::start(3_000);
    let request = server.waiting_request("tokio", 500);
    server.signal("TERM");
    server.wait_for_shutdown_log();
    server.assert_port_closed();
    assert!(response(request).starts_with("HTTP/1.1 200"));
    assert_eq!(server.wait(Duration::from_secs(3)).code(), Some(0));
}

#[test]
fn incomplete_json_cannot_hold_shutdown_past_the_grace_period() {
    let mut server = Server::start(150);
    let _incomplete = server.incomplete_request();
    server.signal("TERM");
    server.wait_for_shutdown_log();
    server.assert_port_closed();
    assert_eq!(server.wait(Duration::from_secs(3)).code(), Some(2));
}

#[test]
fn a_second_signal_exits_without_waiting_for_the_grace_period() {
    let mut server = Server::start(10_000);
    let _incomplete = server.incomplete_request();
    server.signal("INT");
    server.wait_for_shutdown_log();
    server.assert_port_closed();
    server.signal("TERM");
    assert_eq!(server.wait(Duration::from_secs(3)).code(), Some(2));
}

#[test]
fn long_blocking_work_cannot_extend_the_shutdown_deadline() {
    for method in ["thread", "park"] {
        let mut server = Server::start(150);
        let _request = server.waiting_request(method, 10_000);
        server.signal("TERM");
        server.wait_for_shutdown_log();
        server.assert_port_closed();
        assert_eq!(server.wait(Duration::from_secs(3)).code(), Some(2));
    }
}

#[test]
fn a_bind_error_exits_without_leaving_the_supervisor_waiting() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut server = Server::spawn(&listener.local_addr().unwrap().to_string(), 1_000);
    assert_eq!(server.wait(Duration::from_secs(3)).code(), Some(1));
}
