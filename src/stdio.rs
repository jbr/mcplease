//! The stdio transport: newline-delimited JSON-RPC over a byte stream. The
//! public surface is [`calls`], whose docs describe how to drive it; what
//! follows is how it works.
//!
//! [`Calls`] is the whole transport. Polling it reads and decodes lines,
//! answers every request that needs no tool state, records cancellations,
//! admits `tools/call`s, and drives the writer. Each admitted call is handed
//! out as a [`Call`] that owns everything it needs to release, so it is
//! released however the call ends.
//!
//! # Output
//!
//! Every message is serialized whole and queued as one line, and a single
//! writer drains the queue, so the answers and progress notifications of
//! concurrent calls never interleave within a frame. Responses to different
//! ids may be written in an order other than their requests arrived; that is
//! the point, and JSON-RPC correlates by id.
//!
//! The queue is unbounded, because a [`Notifier`] is synchronous and may not
//! wait. Backpressure is applied at the input instead: while the output is
//! backed up, no more lines are read, so a client that stops reading cannot
//! make the server buffer answers to requests it keeps sending.
//!
//! # Cancellation
//!
//! Every call runs inside [`Swansong::interrupt`] on its own child of one
//! parent swansong, so a `notifications/cancelled` drops that call's future at
//! its next await point, and a failed write or a dropped [`Calls`] drops all
//! of them.
//!
//! # Notifications tied to no request
//!
//! [`ConnectionNotifier`] queues onto the same output, so a channel event is
//! one whole frame like any other. It tracks the handshake from the input
//! side: an `initialize` answered with a handshake revision arms it, and the
//! client's `notifications/initialized` opens it. Until then, and for good on
//! a connection that never sends `initialize` — a `2026-07-28` client — it
//! drops what it is given, because those are the only conditions under which
//! the protocol lets a server speak unprompted.
//!
//! # Logging
//!
//! A `notifications/message` passes through a [`LogFilter`] on its way out,
//! whichever sink it was given to. A server that did not declare logging
//! sends none; one that did sends those at or above the level the client last
//! set with `logging/setLevel`, and every level before the client set one.

use crate::{
    server::{ServerConfig, answer_stateless, answer_tool_call},
    traits::{AsToolsList, Dispatch},
    types::{
        JsonRpcError, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
        LATEST_HANDSHAKE_PROTOCOL_VERSION, LoggingLevel, LoggingMessage, Notifier, RequestContext,
        RequestId, SetLevelRequestParams, Tool,
    },
};
use anyhow::{Result, anyhow};
use async_channel::{Receiver, Sender, WeakSender, unbounded};
use futures_lite::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, FutureExt, Stream, StreamExt,
    io::BufReader, pin,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    any::Any,
    collections::HashMap,
    fmt::Debug,
    io,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU8, Ordering},
    },
    task::{Context, Poll},
};
use swansong::{Guard, Swansong};

/// How many frames may wait for the writer before reading pauses.
const BACKLOG: usize = 64;

/// Run the stdio transport over `reader` and `writer` — in practice the
/// process's stdin and stdout, made async however the host's runtime does
/// that — and hand back the admitted `tools/call`s as a stream to spawn.
///
/// ```ignore
/// let mut calls = mcplease::calls::<Tools, _>(stdin, stdout, state, &config);
/// while let Some(call) = calls.next().await {
///     executor.spawn(call?).detach();
/// }
/// ```
///
/// That loop is the whole driver. Polling [`Calls`] is what runs the
/// transport: everything other than `tools/call` — `ping`, `tools/list`,
/// `server/discover`, refusals, cancellations — is handled by the stream
/// itself, and each admitted call comes out as a [`Call`], a `Send + 'static`
/// future that runs the tool and queues its response.
///
/// The stream ends only once the input has ended *and* every call it handed
/// out has finished and been written, so a client that closes its end of the
/// input and keeps reading still gets every answer. An input error counts as
/// EOF. If the output fails, the stream yields `Err` once and then ends; the
/// `?` drops [`Calls`], which cancels every call still running, since nothing
/// can carry their answers. Dropping [`Calls`] early cancels them the same
/// way.
///
/// # Spawn every call
///
/// Reading and writing both happen while [`Calls`] is polled, so it must keep
/// being polled for either to move. A host that awaits each call before taking
/// the next — `while let Some(call) = calls.next().await { call?.await }` —
/// stops reading while a tool runs: no pings answered, no cancellations seen,
/// no second call. That is the serial server `2026-07-28` rules out: it
/// requires ids and progress tokens to be unique across *active* requests, and
/// defines `subscriptions/listen` as a request that never returns.
///
/// Tools therefore run alongside each other, and being async, must not block
/// the executor: a tool with a synchronous stretch — heavy computation,
/// `std::fs`, waiting on a subprocess — moves it to a blocking pool.
///
/// # Limits
///
/// A call arriving when [`ServerConfig::max_concurrent_calls`] are already
/// running is refused at once with [`JsonRpcError::server_busy`]. Output is
/// queued without bound, since progress notifications are synchronous and may
/// not wait, but while the output is backed up no more input is read, so a
/// client that stops reading cannot make the server buffer answers to
/// requests it keeps sending.
///
/// A tool that panics is answered with an internal error.
pub fn calls<Tools, State>(
    reader: impl AsyncRead + Send + 'static,
    writer: impl AsyncWrite + Send + 'static,
    state: State,
    config: &ServerConfig,
) -> Calls
where
    Tools: Debug + AsToolsList + Dispatch<State>,
    State: Clone + Send + Sync + 'static,
{
    let (sender, receiver) = unbounded();
    let cancellation = Swansong::new();
    let shared = Arc::new(Shared {
        output: Output(sender.clone()),
        config: config.clone(),
        running: Swansong::new(),
        cancellation: cancellation.clone(),
        registry: Registry::default(),
        handshake: Arc::default(),
        log_filter: Arc::new(LogFilter::new(config.logging())),
    });
    let notifier = shared.connection_notifier();

    // Fixed for the life of the process, so computed once, here.
    let tools = Tools::tools_list();
    let shared_for_reading = Arc::clone(&shared);
    let receive =
        move |line: &str| receive::<Tools, State>(line, &tools, &state, &shared_for_reading);

    Calls {
        reading: Some(Reading {
            lines: Box::pin(BufReader::new(reader).lines()),
            receive: Box::new(receive),
            backlog: sender,
        }),
        writing: Some(Box::pin(write_loop(writer, receiver))),
        running: shared.running.clone(),
        cancellation,
        notifier,
    }
}

/// The admitted calls of a stdio transport, and the transport itself. See
/// [`calls`].
#[must_use = "the transport runs only while `Calls` is polled, and dropping it cancels every call"]
pub struct Calls {
    /// `None` once the input has ended.
    reading: Option<Reading>,
    /// `None` once the output is finished, or has failed.
    writing: Option<Pin<Box<dyn Future<Output = io::Result<()>> + Send>>>,
    running: Swansong,
    cancellation: Swansong,
    notifier: ConnectionNotifier,
}

/// The input half. Dropped whole at EOF, which releases this side's handle on
/// the output queue: once every call has released its own, the queue closes
/// and the writer finishes.
struct Reading {
    lines: Pin<Box<dyn Stream<Item = io::Result<String>> + Send>>,
    receive: Receive,
    /// For its queue length only.
    backlog: Sender<Vec<u8>>,
}

/// Handles one line of input, handing back the call it admits, if any.
type Receive = Box<dyn FnMut(&str) -> Option<Call> + Send>;

impl Debug for Calls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Calls")
            .field("reading", &self.reading.is_some())
            .field("running", &self.running.guard_count())
            .finish_non_exhaustive()
    }
}

impl Calls {
    /// The next admitted call; `Err` if the output has failed; `None` once
    /// the input has ended and every call handed out has been answered.
    pub async fn next(&mut self) -> Option<Result<Call>> {
        StreamExt::next(self).await
    }

    /// This connection's sink for notifications tied to no request, such as
    /// a [`claude_channel`](crate::claude_channel) event, for a host to hand
    /// to whatever produces them. Every call's
    /// [`RequestContext::connection_notifier`] is the same sink.
    ///
    /// It delivers only once the client has completed the `initialize`
    /// handshake on a revision that has one, and drops what it is given
    /// before that; see [`ConnectionNotifier`]. Holding it does not keep the
    /// transport open.
    pub fn notifier(&self) -> ConnectionNotifier {
        self.notifier.clone()
    }
}

impl Stream for Calls {
    type Item = Result<Call>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Call>>> {
        let this = self.get_mut();
        loop {
            let Some(writing) = &mut this.writing else {
                return Poll::Ready(None);
            };

            if let Poll::Ready(result) = writing.as_mut().poll(cx) {
                this.writing = None;
                this.reading = None;
                return Poll::Ready(match result {
                    // The queue closed: input over, every call released.
                    Ok(()) => None,
                    Err(e) => {
                        log::error!("write failed: {e}");
                        // Nothing running can be answered any more.
                        this.cancellation.shut_down();
                        Some(Err(anyhow!("writing output: {e}")))
                    }
                });
            }

            let Some(reading) = &mut this.reading else {
                // Waiting on the calls still running, and on the writer.
                return Poll::Pending;
            };

            // The writer is pending with frames queued, so it is waiting on
            // the output, which will wake this task when it can take more.
            if reading.backlog.len() >= BACKLOG {
                return Poll::Pending;
            }

            match reading.lines.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(line))) => {
                    if let Some(call) = (reading.receive)(&line) {
                        return Poll::Ready(Some(Ok(call)));
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    log::error!("Error reading line: {e}");
                    this.reading = None;
                }
                Poll::Ready(None) => this.reading = None,
            }
        }
    }
}

impl Drop for Calls {
    fn drop(&mut self) {
        self.cancellation.shut_down();
    }
}

/// One admitted `tools/call`: run it to run the tool and queue its response.
///
/// Boxed so that [`Calls`] and `Call` are plain names a host can store and
/// spawn — one allocation per call, beside the parsing and serialization
/// every call already does.
///
/// A tool that panics is answered with an internal error rather than left
/// hanging, so a call's future never panics and never fails; whatever went
/// wrong writing its answer surfaces from [`Calls`].
#[must_use = "a call does nothing unless polled"]
pub struct Call(Pin<Box<dyn Future<Output = ()> + Send>>);

impl Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Call").finish_non_exhaustive()
    }
}

impl Future for Call {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.0.as_mut().poll(cx)
    }
}

/// What the reader and every call share. The last of them to drop closes the
/// output queue.
struct Shared {
    output: Output,
    config: ServerConfig,
    /// One guard per admitted call, so its guard count is the number running.
    /// Never shut down.
    running: Swansong,
    /// The parent of every call's cancellation.
    cancellation: Swansong,
    registry: Registry,
    handshake: Arc<Handshake>,
    /// What `logging/setLevel` asked for on this connection.
    log_filter: Arc<LogFilter>,
}

impl Shared {
    fn connection_notifier(&self) -> ConnectionNotifier {
        ConnectionNotifier {
            output: self.output.0.downgrade(),
            handshake: Arc::clone(&self.handshake),
            log_filter: Arc::clone(&self.log_filter),
        }
    }
}

/// Handle one line of input, answering it directly unless it is an admitted
/// `tools/call`.
fn receive<Tools, State>(
    line: &str,
    tools: &[Tool],
    state: &State,
    shared: &Arc<Shared>,
) -> Option<Call>
where
    Tools: Debug + AsToolsList + Dispatch<State>,
    State: Clone + Send + Sync + 'static,
{
    if line.trim().is_empty() {
        return None;
    }
    log::trace!("<- {line}");

    let request = match serde_json::from_str(line) {
        Ok(JsonRpcMessage::Request(request)) => request,
        Ok(JsonRpcMessage::Notification(notification)) => {
            match notification.method.as_str() {
                // The one client-sent notification the core protocol defines.
                // stdio has no per-request stream to close, so this is how a
                // client withdraws a call here.
                "notifications/cancelled" => {
                    if let Some(id) = cancelled_request_id(&notification) {
                        shared.registry.cancel(&id);
                    }
                }
                // The end of the handshake, on revisions that have one.
                "notifications/initialized" => shared.handshake.complete(),
                method => log::trace!("ignoring client notification {method}"),
            }
            return None;
        }
        Ok(message) => {
            log::trace!("received {message:?}, ignoring");
            return None;
        }
        Err(e) => {
            log::error!("{e:?}");
            return None;
        }
    };

    let config = &shared.config;
    let response = match answer_stateless(&request, tools, config) {
        Some(response) => response,
        // Admission is here, in order with the input, so it never waits on a
        // call to finish, and a slot is taken before the call is handed on.
        None if shared.running.guard_count() >= config.max_concurrent_calls() => {
            log::warn!("at capacity; refusing {:?}", request.id);
            JsonRpcResponse::error(request.id, JsonRpcError::server_busy())
        }
        None if shared.registry.is_live(&request.id) => {
            log::warn!("refusing {:?}: that id is still running", request.id);
            JsonRpcResponse::error(
                request.id,
                JsonRpcError::invalid_request("request id is already in use"),
            )
        }
        None => return Some(start::<Tools, State>(request, state.clone(), shared)),
    };

    if request.method == "initialize" {
        shared.handshake.negotiated(&response);
    }
    if request.method == "logging/setLevel" && response.error.is_none() {
        shared.log_filter.set(request.params.as_ref());
    }
    shared.output.write(&response);
    None
}

/// Build the future for one admitted call. Everything it needs to release —
/// its slot, its registration, its handle on the output — is owned by the
/// future, so it is released however the future ends: finished, cancelled,
/// or dropped unpolled.
fn start<Tools, State>(request: JsonRpcRequest, state: State, shared: &Arc<Shared>) -> Call
where
    Tools: Debug + Dispatch<State>,
    State: Clone + Send + Sync + 'static,
{
    let guard = shared.running.guard();
    let cancellation = shared.cancellation.child();
    let registration = shared.registry.register(&request.id, &cancellation);
    let shared = Arc::clone(shared);

    Call(Box::pin(async move {
        let _guard: Guard = guard;
        let _registration = registration;
        let id = request.id.clone();
        // Rebuilt per request: `2026-07-28` declares capabilities per-request
        // and forbids inferring them from earlier ones.
        let context = RequestContext::from_request(&request)
            .with_notifier(Arc::new(StdioNotifier {
                output: shared.output.0.downgrade(),
                log_filter: Arc::clone(&shared.log_filter),
            }))
            .with_connection_notifier(Arc::new(shared.connection_notifier()))
            .with_cancellation(cancellation.clone());
        let answer = AssertUnwindSafe(cancellation.interrupt(answer_tool_call::<Tools, State>(
            request,
            state,
            context,
            &shared.config,
        )))
        .catch_unwind()
        .await;

        let response = match answer {
            // `None` twice over: interrupted at an await point, or finished
            // and found cancelled after the fact. Either way the spec says
            // say nothing.
            Ok(answer) => answer.flatten(),
            Err(panic) => {
                log::error!("tool call {id:?} panicked: {}", panic_message(&*panic));
                cancellation
                    .state()
                    .is_running()
                    .then(|| JsonRpcResponse::error(id, JsonRpcError::internal("tool panicked")))
            }
        };

        if let Some(response) = response {
            shared.output.write(&response);
        }
    }))
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(non-string panic payload)")
}

/// Request ids currently running, so an arriving `notifications/cancelled`
/// can find one. Bounded by `max_concurrent_calls`, and each entry is removed
/// by the call that made it.
#[derive(Clone, Default)]
struct Registry(Arc<Mutex<HashMap<RequestId, Swansong>>>);

impl Registry {
    fn register(&self, id: &RequestId, cancellation: &Swansong) -> Registration {
        lock(&self.0).insert(id.clone(), cancellation.clone());
        Registration {
            registry: self.clone(),
            id: id.clone(),
            cancellation: cancellation.clone(),
        }
    }

    /// Whether `id` names a call that is running and not cancelled — which a
    /// new request may not reuse.
    fn is_live(&self, id: &RequestId) -> bool {
        lock(&self.0)
            .get(id)
            .is_some_and(|cancellation| cancellation.state().is_running())
    }

    /// Cancel a running call. Unknown ids — a call that already finished, or
    /// one this server never saw — are ignored, as the spec directs.
    fn cancel(&self, id: &RequestId) {
        match lock(&self.0).get(id) {
            Some(cancellation) => {
                log::debug!("cancelling {id:?}");
                cancellation.shut_down();
            }
            None => log::debug!("ignoring cancellation of unknown request {id:?}"),
        }
    }
}

/// A call's entry in the [`Registry`], removed when the call ends — but only
/// if it is still that call's. A client that reuses the id of a call it
/// cancelled, before that call has noticed, replaces the entry, and the old
/// call must not take the new one's with it.
struct Registration {
    registry: Registry,
    id: RequestId,
    cancellation: Swansong,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut registry = lock(&self.registry.0);
        if registry.get(&self.id) == Some(&self.cancellation) {
            registry.remove(&self.id);
        }
    }
}

/// The request id a `notifications/cancelled` names, if that is what this is.
fn cancelled_request_id(notification: &JsonRpcNotification) -> Option<RequestId> {
    notification
        .params
        .as_ref()
        .and_then(|params| params.get("requestId"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

/// Write each queued frame, flushing whenever the queue runs dry. Finishes
/// when the queue closes — every handle on it dropped — or on the first
/// failed write.
async fn write_loop(writer: impl AsyncWrite, frames: Receiver<Vec<u8>>) -> io::Result<()> {
    pin!(writer);
    while let Ok(frame) = frames.recv().await {
        writer.write_all(&frame).await?;
        if frames.is_empty() {
            writer.flush().await?;
        }
    }
    writer.flush().await
}

/// The queue every message goes through.
struct Output(Sender<Vec<u8>>);

impl Output {
    fn write(&self, message: &impl Serialize) {
        queue(&self.0, message);
    }
}

/// Serialize first, then queue the whole line, so a message from any source
/// is one uninterrupted frame. A closed queue means the writer has failed,
/// which is reported from [`Calls`].
fn queue(sender: &Sender<Vec<u8>>, message: &impl Serialize) {
    match serde_json::to_vec(message) {
        Ok(mut line) => {
            log::trace!("-> {}", String::from_utf8_lossy(&line));
            line.push(b'\n');
            let _ = sender.try_send(line);
        }
        Err(e) => log::error!("serializing output: {e}"),
    }
}

/// The sink a tool reports progress through. It goes out the same way
/// responses do, so a notification emitted mid-call is one whole frame
/// between two others.
///
/// Weak, so that a context a tool clones and keeps past its call cannot hold
/// the output open — and with it the end of [`Calls`].
struct StdioNotifier {
    output: WeakSender<Vec<u8>>,
    log_filter: Arc<LogFilter>,
}

impl Notifier for StdioNotifier {
    fn notify(&self, notification: JsonRpcNotification) {
        if !self.log_filter.admits(&notification) {
            return;
        }
        if let Some(sender) = self.output.upgrade() {
            queue(&sender, &notification);
        }
    }
}

/// The sink for notifications tied to no request: what a
/// [`claude_channel`](crate::claude_channel) event is sent through. From
/// [`Calls::notifier`], or from any call's
/// [`RequestContext::connection_notifier`].
///
/// Revisions through `2025-11-25` let a server send these once the client has
/// sent `notifications/initialized`; `2026-07-28` has no handshake and no such
/// notifications. So this delivers only on a connection whose `initialize`
/// was answered with a handshake revision and then completed, and drops — with
/// a debug log — what it is given before that or on any other connection.
/// Nothing is buffered for later: a notification is about the moment it is
/// sent.
///
/// Weak, like the per-call sink: holding one does not keep the output open.
#[derive(Clone)]
pub struct ConnectionNotifier {
    output: WeakSender<Vec<u8>>,
    handshake: Arc<Handshake>,
    log_filter: Arc<LogFilter>,
}

impl ConnectionNotifier {
    /// Send a log message, if the server declared logging
    /// ([`ServerConfig::with_logging`]) and the message is at or above the
    /// level the client last set with `logging/setLevel` — every level, until
    /// it sets one. Otherwise, and before the handshake completes, it is
    /// dropped, as anything this sink is given is.
    pub fn log(&self, message: LoggingMessage) {
        self.notify(message.into_notification());
    }

    /// The level the client last asked for with `logging/setLevel`, if it has.
    pub fn log_level(&self) -> Option<LoggingLevel> {
        self.log_filter.level()
    }

    /// Whether this sink delivers yet: the client has completed the
    /// `initialize` handshake on a revision that has one. Until then — and
    /// for good on a `2026-07-28` connection — what it is given is dropped,
    /// so a host holding messages for the client can wait for this.
    pub fn is_open(&self) -> bool {
        self.handshake.is_complete()
    }
}

impl Debug for ConnectionNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionNotifier")
            .field("open", &self.handshake.is_complete())
            .finish_non_exhaustive()
    }
}

impl Notifier for ConnectionNotifier {
    fn notify(&self, notification: JsonRpcNotification) {
        if !self.log_filter.admits(&notification) {
            return;
        }
        if !self.handshake.is_complete() {
            log::debug!(
                "dropping {}: the client has not completed a handshake",
                notification.method
            );
            return;
        }
        if let Some(sender) = self.output.upgrade() {
            queue(&sender, &notification);
        }
    }
}

/// Which `notifications/message` a connection may carry: none from a server
/// that did not declare logging, and otherwise those at or above the level the
/// client last set — all of them until it sets one.
struct LogFilter {
    declared: bool,
    level: Mutex<Option<LoggingLevel>>,
}

impl LogFilter {
    fn new(declared: bool) -> Self {
        Self {
            declared,
            level: Mutex::new(None),
        }
    }

    /// Record a `logging/setLevel` that [`answer_stateless`] accepted.
    fn set(&self, params: Option<&Value>) {
        let params = params
            .cloned()
            .and_then(|params| serde_json::from_value::<SetLevelRequestParams>(params).ok());
        if let Some(params) = params {
            *lock(&self.level) = Some(params.level);
        }
    }

    fn level(&self) -> Option<LoggingLevel> {
        *lock(&self.level)
    }

    /// Whether `notification` may be sent. Anything that is not a log
    /// message may.
    fn admits(&self, notification: &JsonRpcNotification) -> bool {
        if notification.method != LoggingMessage::METHOD {
            return true;
        }
        if !self.declared {
            log::debug!("dropping a log message: the server did not declare logging");
            return false;
        }
        match (LoggingMessage::level_of(notification), self.level()) {
            (Some(level), Some(threshold)) => level >= threshold,
            _ => true,
        }
    }
}

/// Where the `initialize` handshake stands, as seen from the input.
#[derive(Default)]
struct Handshake(AtomicU8);

impl Handshake {
    /// …and the client has sent `notifications/initialized`.
    const COMPLETE: u8 = 2;
    /// `initialize` was answered with a handshake revision.
    const NEGOTIATED: u8 = 1;
    const NONE: u8 = 0;

    /// Record the answer to an `initialize`. A revision past the newest
    /// handshake one has no connection-level notifications, so it leaves the
    /// handshake where it was.
    fn negotiated(&self, response: &JsonRpcResponse) {
        let version = response
            .result
            .as_ref()
            .and_then(|result| result.get("protocolVersion"))
            .and_then(Value::as_str);
        // Revisions are ISO dates, so they order as strings.
        if version.is_some_and(|version| version <= LATEST_HANDSHAKE_PROTOCOL_VERSION) {
            let _ = self.0.compare_exchange(
                Self::NONE,
                Self::NEGOTIATED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }

    /// The client's `notifications/initialized`. Ignored unless an
    /// `initialize` came first.
    fn complete(&self) {
        let _ = self.0.compare_exchange(
            Self::NEGOTIATED,
            Self::COMPLETE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn is_complete(&self) -> bool {
        self.0.load(Ordering::Acquire) == Self::COMPLETE
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}
