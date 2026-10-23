//! Notifications tied to no request — a Claude Code channel event — and the
//! handshake that decides whether the connection may carry them.

use super::*;
use mcplease::{
    claude_channel::{self, ChannelEvent},
    types::Notifier,
};

fn initialized() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// `initialize` on `version`, then `notifications/initialized`, answered.
fn handshake(client: &mut pipe::Client, version: &str) {
    client.send(&request(
        "init",
        "initialize",
        json!({"protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "c", "version": "1"}}),
    ));
    let answer = client.next();
    assert_eq!(answer["id"], "init", "{answer}");
    client.send(&initialized());
}

fn assert_channel_event(message: &Value, content: &str) {
    assert_eq!(message["method"], claude_channel::METHOD, "{message}");
    assert_eq!(message["params"]["content"], content);
    assert!(message.get("id").is_none(), "a notification has no id");
}

#[test]
fn an_experimental_capability_is_advertised_by_both_entry_points() {
    let mut server =
        Server::start(config().with_experimental_capability(claude_channel::CAPABILITY, json!({})));
    server.client.send(&request(
        1,
        "initialize",
        json!({"protocolVersion": "2025-11-25"}),
    ));
    server
        .client
        .send(&request(2, "server/discover", json!({})));

    for _ in 0..2 {
        let answer = server.client.next();
        let capabilities = &answer["result"]["capabilities"];
        assert_eq!(
            capabilities["experimental"],
            json!({"claude/channel": {}}),
            "{answer}"
        );
        assert!(capabilities["tools"].is_object(), "{answer}");
    }

    server.join().unwrap();
}

#[test]
fn no_experimental_key_is_written_when_none_is_configured() {
    let mut server = Server::start(config());
    server
        .client
        .send(&request(1, "server/discover", json!({})));
    let answer = server.client.next();
    assert!(
        answer["result"]["capabilities"]
            .get("experimental")
            .is_none(),
        "{answer}"
    );
    server.join().unwrap();
}

/// The output is one FIFO queue, so a notification that had been queued
/// would arrive before the response to anything sent after it.
#[test]
fn a_tool_can_announce_once_the_handshake_completes() {
    let mut server = Server::start(config());
    handshake(&mut server.client, "2025-11-25");
    server
        .client
        .send(&call(1, "announce", json!({"content": "build failed"})));

    let event = server.client.next();
    assert_channel_event(&event, "build failed");
    assert_eq!(event["params"]["meta"], json!({"source_id": "7"}));
    assert_eq!(text(&server.client.next()), "announced");

    server.join().unwrap();
}

#[test]
fn the_host_can_announce_outside_any_call() {
    let mut server = Server::start(config());
    handshake(&mut server.client, "2025-06-18");
    // Answered, so `initialized` — sent before it — has been read.
    server.client.send(&request(1, "ping", json!({})));
    assert_eq!(server.client.next()["id"], 1);

    server
        .notifier
        .notify(ChannelEvent::new("from the host").into_notification());
    assert_channel_event(&server.client.next(), "from the host");

    server.join().unwrap();
}

#[test]
fn nothing_is_announced_before_the_handshake_or_held_for_after_it() {
    let mut server = Server::start(config());
    server
        .client
        .send(&call(1, "announce", json!({"content": "too early"})));
    assert_eq!(text(&server.client.next()), "announced");

    // `initialize` alone arms it; only `initialized` opens it.
    server.client.send(&request(
        "init",
        "initialize",
        json!({"protocolVersion": "2025-11-25"}),
    ));
    assert_eq!(server.client.next()["id"], "init");
    server
        .client
        .send(&call(2, "announce", json!({"content": "still early"})));
    assert_eq!(text(&server.client.next()), "announced");

    server.client.send(&initialized());
    server
        .client
        .send(&call(3, "announce", json!({"content": "now"})));
    assert_channel_event(&server.client.next(), "now");
    assert_eq!(text(&server.client.next()), "announced");

    server.join().unwrap();
}

/// `2026-07-28` has no notifications tied to no request, whether the client
/// skips `initialize` as that revision intends or sends one asking for it.
#[test]
fn a_stateless_connection_never_announces() {
    let mut server = Server::start(config());
    server.client.send(&initialized());
    server
        .client
        .send(&call(1, "announce", json!({"content": "unsent"})));
    assert_eq!(text(&server.client.next()), "announced");

    handshake(&mut server.client, "2026-07-28");
    server
        .client
        .send(&call(2, "announce", json!({"content": "unsent"})));
    assert_eq!(text(&server.client.next()), "announced");

    server.join().unwrap();
}
