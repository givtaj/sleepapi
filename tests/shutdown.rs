#![cfg(unix)]

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::fd::AsRawFd,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    logs: Receiver<String>,
    address: SocketAddr,
    // Keep the read end open without consuming data in backpressure tests.
    _blocked_stdout: Option<io::PipeReader>,
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
        let mut child = Self::command(address, grace_ms)
            .stdout(Stdio::piped())
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
            _blocked_stdout: None,
        }
    }

    fn command(address: &str, grace_ms: u64) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sleepapi"));
        command
            .env("SLEEPAPI_ADDR", address)
            .env("SLEEPAPI_MAX_DURATION_MS", "10000")
            .env("SLEEPAPI_MAX_IN_FLIGHT", "1")
            .env("SLEEPAPI_SHUTDOWN_GRACE_MS", grace_ms.to_string())
            .env("SLEEPAPI_CORS_ORIGINS", "")
            .env("TOKIO_WORKER_THREADS", "2")
            .env("RUST_LOG", "sleepapi=info")
            .stderr(Stdio::inherit());
        command
    }

    fn start_with_blocked_stdout(grace_ms: u64) -> Self {
        let (mut stdout, mut filler) = io::pipe().unwrap();
        set_nonblocking(&stdout, true);
        let child = Self::command("127.0.0.1:0", grace_ms)
            .stdout(filler.try_clone().unwrap())
            .spawn()
            .expect("start sleepapi with controlled stdout");
        let (_, logs) = mpsc::channel();
        // Construct the cleanup guard before doing anything that may fail.
        let mut server = Self {
            child,
            logs,
            address: "127.0.0.1:0".parse().unwrap(),
            _blocked_stdout: None,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut startup = Vec::new();
        while !startup.contains(&b'\n') {
            assert!(Instant::now() < deadline, "startup log timed out");
            let mut bytes = [0; 4096];
            match stdout.read(&mut bytes) {
                Ok(0) => panic!("server exited before announcing its address"),
                Ok(count) => startup.extend_from_slice(&bytes[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => panic!("read startup log: {error}"),
            }
        }
        let startup = String::from_utf8(startup).unwrap();
        let port = startup
            .split_once("127.0.0.1:")
            .expect("startup log should contain the loopback address")
            .1
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap();
        server.address.set_port(port);

        // Saturate the actual pipe, independent of platform capacity or log
        // formatting. No requests are running while O_NONBLOCK is temporarily
        // set on the writer shared with the child. Restore blocking mode before
        // exercising requests or shutdown, and never read stdout again.
        set_nonblocking(&filler, true);
        let mut filled = 0;
        for bytes in [&[b'x'; 4096][..], &b"x"[..]] {
            loop {
                match filler.write(bytes) {
                    Ok(0) => panic!("stdout pipe stopped accepting data"),
                    Ok(count) => filled += count,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => panic!("fill stdout pipe: {error}"),
                }
                assert!(filled < 16 * 1024 * 1024, "stdout pipe did not fill");
            }
        }
        assert!(filled > 0);
        set_nonblocking(&filler, false);
        server._blocked_stdout = Some(stdout);
        server
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

fn set_nonblocking(fd: &impl AsRawFd, enabled: bool) {
    // SAFETY: fd remains owned and open for both fcntl calls; these commands
    // only read or update its file status flags and do not access memory.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0, "get pipe flags: {}", io::Error::last_os_error());
    let flags = if enabled {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
    let result = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags) };
    assert_eq!(result, 0, "set pipe flags: {}", io::Error::last_os_error());
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

#[test]
fn blocked_stdout_does_not_stall_requests_or_shutdown() {
    let mut server = Server::start_with_blocked_stdout(150);
    // Exercise overflow as well as the first blocked write. This exceeds the
    // documented 1024-record logging queue while the OS pipe stays full.
    for _ in 0..2048 {
        assert!(response(server.request("tokio", 0)).starts_with("HTTP/1.1 200"));
    }
    server.signal("TERM");
    // Queued logs cannot flush, so the supervisor must bound logger teardown
    // with the same deadline even though all HTTP requests finished.
    assert_eq!(server.wait(Duration::from_secs(1)).code(), Some(2));
}

#[test]
fn blocked_stdout_cannot_extend_the_shutdown_deadline() {
    let mut server = Server::start_with_blocked_stdout(150);
    let _incomplete = server.incomplete_request();
    server.signal("TERM");
    assert_eq!(server.wait(Duration::from_secs(1)).code(), Some(2));
}

#[test]
fn a_second_signal_exits_with_blocked_stdout() {
    let mut server = Server::start_with_blocked_stdout(10_000);
    let _incomplete = server.incomplete_request();
    server.signal("INT");
    // Closing the listener confirms that the first signal reached the service;
    // waiting for a log line would drain the condition under test.
    server.assert_port_closed();
    assert!(server.child.try_wait().unwrap().is_none());
    server.signal("TERM");
    assert_eq!(server.wait(Duration::from_secs(1)).code(), Some(2));
}
