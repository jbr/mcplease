//! The `logging` capability: declared on request, `logging/setLevel`
//! answered, and log messages filtered by the level the client set.

use super::*;
use mcplease::types::{LoggingLevel, LoggingMessage};

/// `initialize` on a handshake revision, then `notifications/initialized`,
/// then a ping answered — so the handshake has been read before the test
/// sends anything through the connection's sink.
fn handshake(client: &mut pipe::Client) {
    client.send(&request(
        "init",
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "c", "version": "1"}}),
    ));
    assert_eq!(client.next()["id"], "init");
    client.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    client.send(&request("synced", "ping", json!({})));
    assert_eq!(client.next()["id"], "synced");
}

fn message(level: LoggingLevel, text: &str) -> LoggingMessage {
    LoggingMessage::new(level, text).with_logger("test")
}

/// Nothing queued since the last answer: a ping sent now is the next frame.
fn assert_nothing_queued(client: &mut pipe::Client, id: &str) {
    client.send(&request(id, "ping", json!({})));
    let next = client.next();
    assert_eq!(next["id"], id, "a frame was queued: {next}");
}

#[test]
fn logging_is_declared_by_both_entry_points_only_when_configured() {
    let mut server = Server::start(config().with_logging());
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
        assert_eq!(
            answer["result"]["capabilities"]["logging"],
            json!({}),
            "{answer}"
        );
    }
    server.join().unwrap();

    let mut server = Server::start(config());
    server
        .client
        .send(&request(1, "server/discover", json!({})));
    let answer = server.client.next();
    assert!(
        answer["result"]["capabilities"].get("logging").is_none(),
        "{answer}"
    );
    server.join().unwrap();
}

#[test]
fn set_level_is_answered_validated_or_unknown() {
    let mut server = Server::start(config().with_logging());
    server
        .client
        .send(&request(1, "logging/setLevel", json!({"level": "warning"})));
    let answer = server.client.next();
    assert_eq!(answer["result"], json!({}), "{answer}");

    server
        .client
        .send(&request(2, "logging/setLevel", json!({"level": "loud"})));
    let answer = server.client.next();
    assert_eq!(
        answer["error"]["code"],
        error_codes::INVALID_PARAMS,
        "{answer}"
    );
    // A refused level does not replace the accepted one.
    assert_eq!(server.notifier.log_level(), Some(LoggingLevel::Warning));
    server.join().unwrap();

    let mut server = Server::start(config());
    server
        .client
        .send(&request(1, "logging/setLevel", json!({"level": "warning"})));
    let answer = server.client.next();
    assert_eq!(
        answer["error"]["code"],
        error_codes::METHOD_NOT_FOUND,
        "{answer}"
    );
    server.join().unwrap();
}

#[test]
fn log_messages_below_the_set_level_are_dropped() {
    let mut server = Server::start(config().with_logging());
    assert!(!server.notifier.is_open(), "no handshake yet");
    handshake(&mut server.client);
    assert!(server.notifier.is_open());

    // Before any setLevel, every level is sent.
    server.notifier.log(message(LoggingLevel::Debug, "chatter"));
    let sent = server.client.next();
    assert_eq!(sent["method"], "notifications/message", "{sent}");
    assert_eq!(
        sent["params"],
        json!({"level": "debug", "logger": "test", "data": "chatter"})
    );

    server
        .client
        .send(&request(1, "logging/setLevel", json!({"level": "warning"})));
    assert_eq!(server.client.next()["id"], 1);

    server.notifier.log(message(LoggingLevel::Info, "quiet"));
    assert_nothing_queued(&mut server.client, "after-info");

    server.notifier.log(message(LoggingLevel::Warning, "at"));
    server.notifier.log(message(LoggingLevel::Error, "above"));
    assert_eq!(server.client.next()["params"]["data"], "at");
    assert_eq!(server.client.next()["params"]["data"], "above");

    server.join().unwrap();
}

#[test]
fn a_server_that_did_not_declare_logging_sends_no_log_messages() {
    let mut server = Server::start(config());
    handshake(&mut server.client);
    server
        .notifier
        .log(message(LoggingLevel::Error, "undeclared"));
    assert_nothing_queued(&mut server.client, "after");
    server.join().unwrap();
}
