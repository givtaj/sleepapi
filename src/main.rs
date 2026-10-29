use std::{io, thread, time::Duration};

use sleepapi::{
    Config, router,
    transport::{FaultConnection, FaultListener},
};
use tokio::{runtime::Builder, sync::oneshot};
use tracing_appender::non_blocking::NonBlockingBuilder;
use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(supervise(config))?;
    Ok(())
}

async fn supervise(config: Config) -> io::Result<()> {
    // Keep these listeners alive for both the first and second signals.
    let mut signals = ShutdownSignals::new()?;
    let grace = Duration::from_millis(config.shutdown_grace_ms);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (finished_tx, mut finished_rx) = oneshot::channel();

    // The control runtime must stay responsive while the service runtime drains
    // already-started blocking work and flushes logs, either of which can block.
    let service = thread::Builder::new()
        .name("sleepapi-service".into())
        .spawn(move || {
            let result = serve(config, shutdown_rx);
            let _ = finished_tx.send(result);
        })?;

    tokio::select! {
        result = &mut finished_rx => return finish_service(service, result),
        signal = signals.recv() => signal?,
    }

    let deadline = tokio::time::Instant::now() + grace;
    let _ = shutdown_tx.send(());
    tracing::info!(
        grace_ms = grace.as_millis(),
        "shutdown requested; draining in-flight work"
    );

    tokio::select! {
        biased;
        result = &mut finished_rx => {
            // The service must finish inside the original grace period.
            if tokio::time::Instant::now() >= deadline {
                std::process::exit(2);
            }
            finish_service(service, result)
        },
        signal = signals.recv() => {
            signal?;
            std::process::exit(2);
        }
        () = tokio::time::sleep_until(deadline) => {
            std::process::exit(2);
        }
    }
}

fn serve(config: Config, shutdown_rx: oneshot::Receiver<()>) -> io::Result<()> {
    // Both initialization and WorkerGuard destruction belong to this thread.
    // Guard destruction may block on output, even with a lossy queue. Never
    // move it onto the supervisor or flush it after the supervisor has stopped.
    let (log_writer, log_guard) = NonBlockingBuilder::default()
        .buffered_lines_limit(1024)
        .lossy(true)
        .thread_name("sleepapi-logs")
        .finish(logging_stdout()?);
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "sleepapi=info".into()),
        )
        .with_writer(log_writer)
        .init();

    let runtime = Builder::new_multi_thread().enable_all().build()?;
    let result = runtime.block_on(async {
        let app = router(config.clone())?;
        let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
        tracing::info!(
            address = %listener.local_addr()?,
            max_duration_ms = config.max_duration_ms,
            max_in_flight = config.max_in_flight,
            shutdown_grace_ms = config.shutdown_grace_ms,
            "sleepapi listening"
        );
        axum::serve(
            FaultListener::new(listener),
            app.into_make_service_with_connect_info::<FaultConnection>(),
        )
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    // A disconnected request may have left blocking work behind after serve
    // finishes. Wait for it here so exit 0 means every task drained; the separate
    // supervisor still enforces the deadline and second-signal escape, including
    // while the logging guard attempts a best-effort flush.
    drop(runtime);
    drop(log_guard);
    result
}

fn logging_stdout() -> io::Result<impl io::Write + Send + 'static> {
    // Use an owned OS handle so the logging worker does not share Rust's global
    // stdout buffer or its process-exit flushing path.
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        Ok(std::fs::File::from(
            io::stdout().as_fd().try_clone_to_owned()?,
        ))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        Ok(std::fs::File::from(
            io::stdout().as_handle().try_clone_to_owned()?,
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(io::stdout())
    }
}

fn finish_service(
    service: thread::JoinHandle<()>,
    result: Result<io::Result<()>, oneshot::error::RecvError>,
) -> io::Result<()> {
    service
        .join()
        .map_err(|_| io::Error::other("service thread panicked"))?;
    result.map_err(|_| io::Error::other("service thread stopped without a result"))?
}

struct ShutdownSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    interrupt: tokio::signal::windows::CtrlC,
}

impl ShutdownSignals {
    fn new() -> io::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            interrupt: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
            #[cfg(unix)]
            terminate: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
            #[cfg(windows)]
            interrupt: tokio::signal::windows::ctrl_c()?,
        })
    }

    async fn recv(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        let received = tokio::select! {
            value = self.interrupt.recv() => value,
            value = self.terminate.recv() => value,
        };
        #[cfg(windows)]
        let received = self.interrupt.recv().await;
        #[cfg(not(any(unix, windows)))]
        let received = {
            tokio::signal::ctrl_c().await?;
            Some(())
        };
        received.ok_or_else(|| io::Error::other("shutdown signal stream closed"))
    }
}
