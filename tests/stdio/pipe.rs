//! In-process stdin/stdout: the client writes lines into the server's input
//! and reads lines from its output, with a switch to make the output fail.

use futures_lite::{AsyncRead, AsyncWrite, Stream};
use serde_json::Value;
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(10);

pub struct ServerEnd {
    pub input: Input,
    pub output: Output,
}

pub struct Client {
    input: Option<async_channel::Sender<Vec<u8>>>,
    output: Receiver<Vec<u8>>,
    pending: Vec<u8>,
    broken: Arc<AtomicBool>,
    reader_exited: Arc<AtomicBool>,
}

pub fn pair() -> (Client, ServerEnd) {
    let (input_tx, input_rx) = async_channel::unbounded();
    let (output_tx, output_rx) = channel();
    let broken = Arc::new(AtomicBool::new(false));
    let reader_exited = Arc::new(AtomicBool::new(false));
    let client = Client {
        input: Some(input_tx),
        output: output_rx,
        pending: vec![],
        broken: broken.clone(),
        reader_exited: reader_exited.clone(),
    };
    let server = ServerEnd {
        input: Input {
            rx: Box::pin(input_rx),
            pending: vec![],
            exited: reader_exited,
        },
        output: Output {
            tx: output_tx,
            broken,
        },
    };
    (client, server)
}

impl Client {
    pub fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        self.input
            .as_ref()
            .expect("input closed")
            .try_send(line)
            .expect("server input gone");
    }

    /// The next response, parsed. Panics rather than waiting forever.
    pub fn next(&mut self) -> Value {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(newline) = self.pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=newline).collect();
                return serde_json::from_slice(&line[..line.len() - 1]).unwrap_or_else(|e| {
                    panic!("torn frame: {e}: {}", String::from_utf8_lossy(&line))
                });
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(remaining) {
                Ok(chunk) => self.pending.extend(chunk),
                Err(RecvTimeoutError::Timeout) => panic!("no response within {TIMEOUT:?}"),
                Err(RecvTimeoutError::Disconnected) => panic!("server output closed"),
            }
        }
    }

    /// EOF on the server's stdin.
    pub fn close_input(&mut self) {
        self.input.take();
    }

    /// Every write to the server's stdout fails from now on.
    pub fn break_output(&self) {
        self.broken.store(true, Ordering::SeqCst);
    }

    /// Whether the server has dropped its input.
    pub fn reader_exited(&self) -> bool {
        self.reader_exited.load(Ordering::SeqCst)
    }

    pub fn wait_reader_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.reader_exited() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }
}

/// The server's stdin. Dropped with the transport.
pub struct Input {
    rx: Pin<Box<async_channel::Receiver<Vec<u8>>>>,
    pending: Vec<u8>,
    exited: Arc<AtomicBool>,
}

impl AsyncRead for Input {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if self.pending.is_empty() {
            match self.rx.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(chunk)) => self.pending = chunk,
                Poll::Ready(None) => return Poll::Ready(Ok(0)),
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Poll::Ready(Ok(n))
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.exited.store(true, Ordering::SeqCst);
    }
}

/// The server's stdout.
pub struct Output {
    tx: Sender<Vec<u8>>,
    broken: Arc<AtomicBool>,
}

impl Output {
    fn check(&self) -> io::Result<()> {
        if self.broken.load(Ordering::SeqCst) {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "output broken"))
        } else {
            Ok(())
        }
    }
}

impl AsyncWrite for Output {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.check()?;
        // The client may have gone; a real pipe would say EPIPE, and so do we.
        self.tx
            .send(buf.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "client gone"))?;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.check())
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
