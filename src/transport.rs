//! HTTP/1 fault bodies and the flush-aware listener used to serve them.
//!
//! A truncation is not an early end-of-stream: it advertises the full fixture
//! length, flushes the selected prefix, then fails the body and closes the
//! connection. The flush handshake avoids depending on sleeps or socket-buffer
//! sizes to get the prefix onto the wire before Hyper discards its write buffer.

use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use axum::{
    body::{Body, Bytes},
    extract::connect_info::Connected,
    serve::{IncomingStream, Listener},
};
use http_body::{Body as HttpBody, Frame, SizeHint};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};

/// Listener that preserves Axum's server and graceful-shutdown behavior while
/// exposing successful socket flushes to deliberately truncated response bodies.
pub struct FaultListener {
    inner: TcpListener,
}

impl FaultListener {
    pub fn new(inner: TcpListener) -> Self {
        Self { inner }
    }
}

impl Listener for FaultListener {
    type Io = FaultStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (inner, address) = Listener::accept(&mut self.inner).await;
        (
            FaultStream {
                inner,
                connection: FaultConnection::default(),
            },
            address,
        )
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

#[derive(Default)]
struct FlushState {
    generation: u64,
    waiting: Option<Waker>,
}

/// Per-connection flush synchronization, supplied through Axum `ConnectInfo`.
#[derive(Clone, Default)]
pub struct FaultConnection {
    flush: Arc<Mutex<FlushState>>,
}

impl Connected<IncomingStream<'_, FaultListener>> for FaultConnection {
    fn connect_info(stream: IncomingStream<'_, FaultListener>) -> Self {
        stream.io().connection.clone()
    }
}

/// Pass-through socket IO; only a fully completed flush advances the handshake.
pub struct FaultStream {
    inner: TcpStream,
    connection: FaultConnection,
}

impl AsyncRead for FaultStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buffer)
    }
}

impl AsyncWrite for FaultStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buffer)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffers: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, buffers)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match Pin::new(&mut self.inner).poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                let wake = {
                    let mut flush = self.connection.flush.lock().unwrap();
                    flush.generation = flush.generation.wrapping_add(1);
                    flush.waiting.take()
                };
                if let Some(wake) = wake {
                    wake.wake();
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

type Completion = Box<dyn FnOnce() + Send + 'static>;

enum FaultState {
    Drop,
    Prefix(Bytes),
    AwaitFlush(u64),
    Failed,
}

struct FaultBody {
    state: FaultState,
    connection: Option<FaultConnection>,
    full_length: u64,
    on_injected: Option<Completion>,
}

impl FaultBody {
    fn inject(&mut self) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        self.state = FaultState::Failed;
        if let Some(on_injected) = self.on_injected.take() {
            on_injected();
        }
        Poll::Ready(Some(Err(io::Error::other("intentional transport fault"))))
    }
}

impl HttpBody for FaultBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match &self.state {
            FaultState::Drop => self.inject(),
            FaultState::Prefix(_) => {
                // Sample here, not when constructing the response: earlier
                // responses or idle socket flushes must not satisfy this wait.
                let generation = self
                    .connection
                    .as_ref()
                    .unwrap()
                    .flush
                    .lock()
                    .unwrap()
                    .generation;
                let FaultState::Prefix(prefix) =
                    std::mem::replace(&mut self.state, FaultState::AwaitFlush(generation))
                else {
                    unreachable!()
                };
                self.full_length -= prefix.len() as u64;
                Poll::Ready(Some(Ok(Frame::data(prefix))))
            }
            FaultState::AwaitFlush(generation) => {
                let flushed = {
                    let mut flush = self.connection.as_ref().unwrap().flush.lock().unwrap();
                    if flush.generation != *generation {
                        true
                    } else {
                        flush.waiting = Some(cx.waker().clone());
                        false
                    }
                };
                if flushed {
                    self.inject()
                } else {
                    Poll::Pending
                }
            }
            FaultState::Failed => Poll::Ready(None),
        }
    }

    fn is_end_stream(&self) -> bool {
        // Hyper must poll even a header-only drop, and must never interpret a
        // prefix as a successfully completed response body.
        false
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.full_length)
    }
}

/// Fail on the first body poll, before Hyper flushes the buffered response head.
/// Real HTTP/1 TCP regressions pin this behavior against the locked server stack.
pub(crate) fn connection_drop(on_injected: impl FnOnce() + Send + 'static) -> Body {
    Body::new(FaultBody {
        state: FaultState::Drop,
        connection: None,
        // A nonzero hint keeps Hyper from treating this as an empty body.
        full_length: 1,
        on_injected: Some(Box::new(on_injected)),
    })
}

/// Flush `prefix` while declaring `full_length`, then fail the response body.
/// Dropping this body before injection drops the callback without invoking it,
/// allowing its owned attempt/capacity guards to record cancellation and release.
pub(crate) fn truncated_response(
    connection: &FaultConnection,
    prefix: Bytes,
    full_length: u64,
    on_injected: impl FnOnce() + Send + 'static,
) -> Body {
    debug_assert!(!prefix.is_empty() && (prefix.len() as u64) < full_length);
    Body::new(FaultBody {
        state: FaultState::Prefix(prefix),
        connection: Some(connection.clone()),
        full_length,
        on_injected: Some(Box::new(on_injected)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Release(Arc<AtomicUsize>);

    impl Drop for Release {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn body(
        connection: &FaultConnection,
        injected: &Arc<AtomicUsize>,
        released: &Arc<AtomicUsize>,
    ) -> FaultBody {
        let injected = injected.clone();
        let release = Release(released.clone());
        FaultBody {
            state: FaultState::Prefix(Bytes::from_static(b"abc")),
            connection: Some(connection.clone()),
            full_length: 6,
            on_injected: Some(Box::new(move || {
                injected.fetch_add(1, Ordering::SeqCst);
                drop(release);
            })),
        }
    }

    #[test]
    fn truncation_waits_for_a_flush_after_yielding_the_prefix() {
        let connection = FaultConnection::default();
        let injected = Arc::new(AtomicUsize::new(0));
        let released = Arc::new(AtomicUsize::new(0));
        let mut body = body(&connection, &injected, &released);
        let mut cx = Context::from_waker(Waker::noop());
        // A flush that occurs after construction but before prefix polling is
        // stale: it must never permit the deliberate body error.
        connection.flush.lock().unwrap().generation = 42;
        assert_eq!(body.size_hint().exact(), Some(6));
        let Poll::Ready(Some(Ok(prefix))) = Pin::new(&mut body).poll_frame(&mut cx) else {
            panic!("first body frame must be the prefix");
        };
        assert_eq!(prefix.into_data().unwrap(), b"abc".as_slice());
        assert_eq!(body.size_hint().exact(), Some(3));
        for _ in 0..3 {
            assert!(Pin::new(&mut body).poll_frame(&mut cx).is_pending());
        }
        assert_eq!(injected.load(Ordering::SeqCst), 0);
        assert_eq!(released.load(Ordering::SeqCst), 0);
        connection.flush.lock().unwrap().generation = 43;
        assert!(matches!(
            Pin::new(&mut body).poll_frame(&mut cx),
            Poll::Ready(Some(Err(_)))
        ));
        assert_eq!(injected.load(Ordering::SeqCst), 1);
        assert_eq!(released.load(Ordering::SeqCst), 1);
        assert!(matches!(
            Pin::new(&mut body).poll_frame(&mut cx),
            Poll::Ready(None)
        ));
        drop(body);
        assert_eq!(injected.load(Ordering::SeqCst), 1);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_a_fault_body_releases_its_guards_without_claiming_injection() {
        for poll_prefix in [false, true] {
            let connection = FaultConnection::default();
            let injected = Arc::new(AtomicUsize::new(0));
            let released = Arc::new(AtomicUsize::new(0));
            let mut body = body(&connection, &injected, &released);
            if poll_prefix {
                let mut cx = Context::from_waker(Waker::noop());
                assert!(Pin::new(&mut body).poll_frame(&mut cx).is_ready());
                assert!(Pin::new(&mut body).poll_frame(&mut cx).is_pending());
            }
            drop(body);
            assert_eq!(injected.load(Ordering::SeqCst), 0);
            assert_eq!(released.load(Ordering::SeqCst), 1);
        }
    }
}
