//! The stdio transport, driven over in-process pipes: what a client on the
//! other end of stdin and stdout observes.
//!
//! Two properties are under test. Protocol traffic keeps moving while tools
//! run, and tools run *alongside each other* — which is what the protocol
//! asks for and what `subscriptions/listen` will need. Every tool that needs
//! to be "slow" here parks on a oneshot the test holds, so nothing depends on
//! timing: a response that arrives before the release is sent arrived while
//! the tool was genuinely outstanding.

#![cfg(feature = "stdio")]

mod connection;
mod logging;
mod pipe;

use anyhow::Result;
use async_channel::{Receiver as Parked, Sender as Release, bounded};
use async_executor::Executor;
use futures_lite::future::block_on;
use mcplease::{
    Calls, ConnectionNotifier, ServerConfig, calls,
    types::{Implementation, error_codes},
};
use pipe::{Client, ServerEnd};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// Shared, cloned per call — which is the shape `2026-07-28` expects, since a
/// server holds resources rather than a conversation.
#[derive(Clone)]
pub struct State {
    pub started: Sender<()>,
    waiters: Arc<Mutex<VecDeque<Release<()>>>>,
    log: Arc<Mutex<Vec<String>>>,
}

impl State {
    /// Register to be released, and hand back the future to await.
    fn park(&self) -> Parked<()> {
        let (sender, receiver) = bounded(1);
        self.waiters.lock().unwrap().push_back(sender);
        receiver
    }

    /// Append to the shared log and report the new length.
    fn record(&self, entry: &str) -> usize {
        let mut log = self.log.lock().unwrap();
        log.push(entry.to_string());
        log.len()
    }
}

mcplease::tools!(
    cli = false,
    State,
    (Block, block, "block"),
    (Note, note, "note"),
    (Big, big, "big"),
    (Boom, boom, "boom"),
    (Tick, tick, "tick"),
    (Announce, announce, "announce")
);

fn config() -> ServerConfig {
    ServerConfig::new(Implementation::new("test-server", "1.2.3"))
}

/// A server on its own thread, and the client's end of its pipes.
struct Server {
    client: Client,
    started: Receiver<()>,
    waiters: Arc<Mutex<VecDeque<Release<()>>>>,
    notifier: ConnectionNotifier,
    handle: JoinHandle<Result<()>>,
}

impl Server {
    /// A server that spawns every call onto an executor, the way a host does.
    fn start(config: ServerConfig) -> Self {
        Self::start_driven(config, |mut calls| {
            let executor = Executor::new();
            block_on(executor.run(async {
                while let Some(call) = calls.next().await {
                    executor.spawn(call?).detach();
                }
                Ok(())
            }))
        })
    }

    /// A server that gives every call a thread of its own, so a tool can
    /// block without stalling the transport — which is what lets a test hold a
    /// call past its cancellation.
    fn start_threaded(config: ServerConfig) -> Self {
        Self::start_driven(config, |mut calls| {
            block_on(async {
                while let Some(call) = calls.next().await {
                    let call = call?;
                    thread::spawn(move || block_on(call));
                }
                Ok(())
            })
        })
    }

    fn start_driven(
        config: ServerConfig,
        drive: impl FnOnce(Calls) -> Result<()> + Send + 'static,
    ) -> Self {
        let (client, ServerEnd { input, output }) = pipe::pair();
        let (started_tx, started) = mpsc::channel();
        let waiters = Arc::<Mutex<VecDeque<Release<()>>>>::default();
        let state = State {
            started: started_tx,
            waiters: Arc::clone(&waiters),
            log: Arc::default(),
        };
        let calls = calls::<Tools, _>(input, output, state, &config);
        let notifier = calls.notifier();
        let handle = thread::spawn(move || drive(calls));
        Self {
            client,
            started,
            waiters,
            notifier,
            handle,
        }
    }

    /// Wait until one more `block` tool has started and parked.
    fn wait_started(&self) {
        self.started
            .recv_timeout(Duration::from_secs(10))
            .expect("no tool started");
    }

    /// Let the longest-parked `block` tool return.
    fn release(&self) {
        let waiter = loop {
            if let Some(waiter) = self.waiters.lock().unwrap().pop_front() {
                break waiter;
            }
            thread::sleep(Duration::from_millis(5));
        };
        let _ = waiter.try_send(());
    }

    fn join(self) -> Result<()> {
        drop(self.client);
        self.handle.join().expect("serve panicked")
    }
}

fn request(id: impl Into<Value>, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id.into(), "method": method, "params": params})
}

fn call(id: impl Into<Value>, name: &str, arguments: Value) -> Value {
    request(
        id,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
}

/// A `tools/call` that opts in to progress, with the token the spec puts in
/// `_meta` — bare, unlike the `io.modelcontextprotocol/*` keys.
fn call_with_progress(id: impl Into<Value>, name: &str, arguments: Value, token: Value) -> Value {
    request(
        id,
        "tools/call",
        json!({"name": name, "arguments": arguments, "_meta": {"progressToken": token}}),
    )
}

fn cancel(id: impl Into<Value>, reason: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/cancelled",
        "params": {"requestId": id.into(), "reason": reason},
    })
}

fn text(response: &Value) -> &str {
    response["result"]["content"][0]["text"].as_str().unwrap()
}

#[test]
fn ping_is_answered_while_a_tool_call_is_outstanding() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));
    server.client.send(&request(2, "ping", json!({})));

    let ping = server.client.next();
    assert_eq!(ping["id"], 2, "{ping}");
    assert_eq!(ping["result"], json!({}));

    server.release();
    let blocked = server.client.next();
    assert_eq!(blocked["id"], 1, "{blocked}");
    assert_eq!(text(&blocked), "block #1");

    server.join().unwrap();
}

#[test]
fn every_state_independent_method_is_answered_while_a_tool_call_is_outstanding() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));

    server.client.send(&request(
        2,
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
    ));
    server
        .client
        .send(&request(3, "server/discover", json!({})));
    server.client.send(&request(4, "tools/list", json!({})));
    server.client.send(&request(5, "nope", json!({})));

    let initialize = server.client.next();
    assert_eq!(initialize["id"], 2, "{initialize}");
    assert_eq!(initialize["result"]["protocolVersion"], "2025-06-18");

    let discover = server.client.next();
    assert_eq!(discover["id"], 3, "{discover}");
    assert!(discover["result"]["supportedVersions"].is_array());

    let list = server.client.next();
    assert_eq!(list["id"], 4, "{list}");
    let names: Vec<_> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Block", "Note", "Big", "Boom", "Tick", "Announce"]);

    let unknown = server.client.next();
    assert_eq!(unknown["id"], 5, "{unknown}");
    assert_eq!(unknown["error"]["code"], error_codes::METHOD_NOT_FOUND);

    server.release();
    assert_eq!(server.client.next()["id"], 1);
    server.join().unwrap();
}

/// Calls run alongside each other, so a second one does not wait behind a
/// slow first. This is the property `subscriptions/listen` will need, and the
/// one a serialized server cannot offer.
#[test]
fn a_second_tool_call_runs_while_the_first_is_outstanding() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));
    server.wait_started();
    server
        .client
        .send(&call(2, "note", json!({"text": "meanwhile"})));

    let note = server.client.next();
    assert_eq!(note["id"], 2, "{note}");
    assert_eq!(text(&note), "meanwhile #1");

    server.release();
    let blocked = server.client.next();
    assert_eq!(blocked["id"], 1, "{blocked}");
    assert_eq!(text(&blocked), "block #2");

    server.join().unwrap();
}

/// Invalid `tools/call` params are decided where the tool would have run, but
/// that no longer means waiting for an unrelated tool.
#[test]
fn a_malformed_tool_call_is_refused_without_waiting() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));
    server.wait_started();
    server.client.send(&call(2, "no-such-tool", json!({})));

    let bad = server.client.next();
    assert_eq!(bad["id"], 2, "{bad}");
    assert_eq!(bad["error"]["code"], error_codes::INVALID_PARAMS);

    server.release();
    assert_eq!(server.client.next()["id"], 1);
    server.join().unwrap();
}

#[test]
fn calls_past_the_concurrency_limit_are_refused_promptly_and_ping_still_answers() {
    let limit = 3;
    let mut server = Server::start(config().with_max_concurrent_calls(limit));

    for id in 1..=limit {
        server.client.send(&call(id, "block", json!({})));
        server.wait_started();
    }
    server.client.send(&call(limit + 1, "block", json!({})));
    server.client.send(&request("ping", "ping", json!({})));

    let busy = server.client.next();
    assert_eq!(busy["id"], limit + 1, "{busy}");
    assert_eq!(busy["error"]["code"], error_codes::SERVER_BUSY);

    let ping = server.client.next();
    assert_eq!(ping["id"], "ping", "{ping}");

    // Released one at a time, so the log order — and therefore each
    // response's number — is the release order.
    for id in 1..=limit {
        server.release();
        let response = server.client.next();
        assert_eq!(response["id"], id, "{response}");
        assert_eq!(text(&response), format!("block #{id}"));
    }

    // A slot freed is a slot reusable.
    server
        .client
        .send(&call("after", "note", json!({"text": "after"})));
    assert_eq!(server.client.next()["id"], "after");

    server.join().unwrap();
}

#[test]
fn progress_notifications_carry_the_token_and_precede_the_response() {
    let mut server = Server::start(config());
    server.client.send(&call_with_progress(
        1,
        "tick",
        json!({"count": 3}),
        json!("abc123"),
    ));

    for step in 1..=3 {
        let note = server.client.next();
        assert_eq!(note["method"], "notifications/progress", "{note}");
        assert_eq!(note["params"]["progressToken"], "abc123");
        assert_eq!(note["params"]["progress"].as_f64(), Some(f64::from(step)));
        assert_eq!(note["params"]["total"].as_f64(), Some(3.0));
        assert_eq!(note["params"]["message"], "ticking");
        assert!(note.get("id").is_none(), "a notification has no id");
    }

    let response = server.client.next();
    assert_eq!(response["id"], 1, "{response}");
    assert_eq!(text(&response), "ticked 3");

    server.join().unwrap();
}

/// Without a token there is nothing to correlate a notification with, so the
/// spec's fallback — say nothing — is what a tool's `progress` call does.
#[test]
fn progress_is_silent_when_the_client_sent_no_token() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "tick", json!({"count": 3})));

    let response = server.client.next();
    assert_eq!(response["id"], 1, "{response}");
    assert_eq!(text(&response), "ticked 3");

    server.join().unwrap();
}

/// A cancelled request gets no response at all — not a result, not an error.
#[test]
fn a_cancelled_call_is_answered_with_nothing() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));
    server.wait_started();

    server.client.send(&cancel(1, "user hit escape"));
    // The reader takes lines in order, so an answered ping proves the
    // cancellation landed before the release below.
    server.client.send(&request(2, "ping", json!({})));
    assert_eq!(server.client.next()["id"], 2);

    server.release();

    // Any response to the cancelled call would be written before this one.
    server.client.send(&request(3, "ping", json!({})));
    let next = server.client.next();
    assert_eq!(next["id"], 3, "the cancelled call was answered: {next}");

    server.join().unwrap();
}

/// A cancelled call's future is dropped where it waits, so an async tool stops
/// without looking — it never gets as far as the work after its await.
#[test]
fn a_cancelled_tool_is_dropped_at_its_await() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));
    server.wait_started();
    server.client.send(&cancel(1, "changed my mind"));
    server.client.send(&request(2, "ping", json!({})));
    assert_eq!(server.client.next()["id"], 2);

    server.release();

    // `block` records to the log only after its release, so a second call's
    // number reveals whether the first got that far.
    server
        .client
        .send(&call(3, "note", json!({"text": "after"})));
    let note = server.client.next();
    assert_eq!(note["id"], 3, "{note}");
    assert_eq!(text(&note), "after #1", "the cancelled tool kept working");

    server.join().unwrap();
}

/// Cancelling something that already finished, or was never seen, is a race
/// the spec says to swallow.
#[test]
fn cancelling_an_unknown_request_is_ignored() {
    let mut server = Server::start(config());
    server
        .client
        .send(&call(1, "note", json!({"text": "done"})));
    assert_eq!(server.client.next()["id"], 1);

    server.client.send(&cancel(1, "too late"));
    server.client.send(&cancel(99, "never existed"));
    server.client.send(&request(2, "ping", json!({})));
    assert_eq!(server.client.next()["id"], 2);

    server.join().unwrap();
}

#[test]
fn responses_and_notifications_are_whole_lines() {
    let mut server = Server::start(config());
    let pings = 200;
    // A response large enough that writing it takes real time while pings
    // and progress notifications contend for the writer.
    server
        .client
        .send(&call("big", "big", json!({"bytes": 4 * 1024 * 1024})));
    server.client.send(&call_with_progress(
        "tick",
        "tick",
        json!({"count": 50}),
        json!(7),
    ));
    for id in 0..pings {
        server.client.send(&request(id, "ping", json!({})));
    }

    // `next` parses each line as JSON, so a torn frame fails right there.
    let mut seen = vec![];
    let mut ticks = 0;
    while ticks < 50 || seen.len() < pings + 2 {
        let message = server.client.next();
        if message["method"] == "notifications/progress" {
            assert_eq!(message["params"]["progressToken"], 7);
            ticks += 1;
            continue;
        }
        if message["id"] == "big" {
            assert_eq!(text(&message).len(), 4 * 1024 * 1024);
        }
        seen.push(message["id"].clone());
    }
    assert!(seen.contains(&json!("big")));
    for id in 0..pings {
        assert!(seen.contains(&json!(id)), "ping {id} unanswered");
    }
    server.join().unwrap();
}

#[test]
fn clean_eof_finishes_every_accepted_call_before_returning() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));
    server.wait_started();
    server
        .client
        .send(&call(2, "note", json!({"text": "second"})));
    server
        .client
        .send(&call(3, "note", json!({"text": "third"})));
    server.client.close_input();

    // The notes were accepted before EOF and do not wait for the block.
    assert_eq!(text(&server.client.next()), "second #1");
    assert_eq!(text(&server.client.next()), "third #2");

    server.release();
    assert_eq!(text(&server.client.next()), "block #3");

    // Nothing more can arrive; the server returned `Ok`.
    assert!(server.client.wait_reader_exit(Duration::from_secs(5)));
    server.join().unwrap();
}

#[test]
fn a_write_failure_ends_the_stream_with_err_and_releases_the_input() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));
    server.wait_started();
    server.client.break_output();
    server.release();

    // The response hit the broken output, the stream yielded `Err`, and the
    // server returned with the input still open — and let go of it.
    let result = server.handle.join().expect("serve panicked");
    assert!(result.is_err(), "{result:?}");
    assert!(server.client.reader_exited());
}

/// A failed write anywhere cancels the calls still running — nothing they
/// produce could be delivered — so the server ends rather than waiting on a
/// call no one will release.
#[test]
fn a_reader_side_write_failure_cancels_running_calls_and_ends_the_server() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));
    server.wait_started();
    server.client.break_output();
    server.client.send(&request(2, "ping", json!({})));

    assert!(server.client.wait_reader_exit(Duration::from_secs(5)));
    let result = server.handle.join().expect("serve panicked");
    assert!(result.is_err(), "{result:?}");
}

/// Ids must be unique among requests still awaiting a response, so a second
/// call reusing a running one's id is refused rather than run.
#[test]
fn a_call_reusing_a_running_id_is_refused() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "block", json!({})));
    server.wait_started();
    server
        .client
        .send(&call(1, "note", json!({"text": "impostor"})));

    let refused = server.client.next();
    assert_eq!(refused["id"], 1, "{refused}");
    assert_eq!(refused["error"]["code"], error_codes::INVALID_REQUEST);

    server.release();
    assert_eq!(text(&server.client.next()), "block #1");
    server.join().unwrap();
}

/// A client that reuses the id of a call it cancelled, before that call has
/// wound down, gets a new call that is itself cancellable: the old call's
/// cleanup must not take the new call's registration with it.
#[test]
fn an_id_reused_after_cancellation_stays_cancellable() {
    // Two slots: the old call and the new one fill them, so a third call
    // being admitted is how the test sees the old call fully wound down.
    let mut server = Server::start_threaded(config().with_max_concurrent_calls(2));
    // Blocking, so it cannot be dropped at an await and outlives its
    // cancellation until released.
    server
        .client
        .send(&call(1, "block", json!({"blocking": true})));
    server.wait_started();
    server.client.send(&cancel(1, "never mind"));
    server.client.send(&call(1, "block", json!({})));
    server.wait_started();

    // The old call ends, discarded; its slot is freed only after its
    // registration is gone, so once a probe is admitted, that cleanup ran.
    server.release();
    let mut probe = 0;
    loop {
        server.client.send(&call(
            format!("probe {probe}"),
            "note",
            json!({"text": "probe"}),
        ));
        let response = server.client.next();
        if response.get("error").is_none() {
            break;
        }
        assert_eq!(response["error"]["code"], error_codes::SERVER_BUSY);
        probe += 1;
        thread::sleep(Duration::from_millis(5));
    }

    // The new call must still be registered, so cancelling it drops it
    // before its release.
    server.client.send(&cancel(1, "never mind again"));
    server.client.send(&request(2, "ping", json!({})));
    assert_eq!(server.client.next()["id"], 2);
    server.release();

    server
        .client
        .send(&call(3, "note", json!({"text": "after"})));
    let note = server.client.next();
    assert_eq!(note["id"], 3, "{note}");
    // Only the probe recorded: the old call saw its cancellation at the
    // progress checkpoint after its release, and the new one was dropped
    // where it waited. A new call that missed its cancellation makes this #3.
    assert_eq!(text(&note), "after #2", "the reused id was not cancellable");
    server.join().unwrap();
}

/// A driver that spawns has handed its calls off by the time the input ends;
/// the stream outliving the input is what keeps their answers coming.
#[test]
fn the_stream_outlives_eof_until_spawned_calls_finish() {
    let mut server = Server::start_threaded(config());
    server
        .client
        .send(&call(1, "block", json!({"blocking": true})));
    server.wait_started();
    server.client.close_input();
    assert!(server.client.wait_reader_exit(Duration::from_secs(5)));
    assert!(!server.handle.is_finished(), "returned with a call running");

    server.release();
    assert_eq!(text(&server.client.next()), "block #1");
    server.join().unwrap();
}

/// A panic is contained to its call, which is answered rather than left for
/// the client to time out.
#[test]
fn a_tool_panic_is_answered_with_an_internal_error() {
    let mut server = Server::start(config());
    server.client.send(&call(1, "boom", json!({})));

    let boom = server.client.next();
    assert_eq!(boom["id"], 1, "{boom}");
    assert_eq!(boom["error"]["code"], error_codes::INTERNAL_ERROR);

    server
        .client
        .send(&call(2, "note", json!({"text": "still here"})));
    assert_eq!(text(&server.client.next()), "still here #1");
    server.join().unwrap();
}
