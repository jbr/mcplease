//! The `tools!` macro and `handle_request`: dispatch as a transport sees it.
//!
//! `handle_request` does no I/O, so the whole server surface is testable by
//! handing it a decoded request — which is also how a non-stdio transport uses
//! it. Nothing here is command-line specific, so this file compiles and passes
//! both with and without the `cli` feature, covering both arms of the
//! `__tools_enum!` split.

#![cfg(feature = "server")]

use mcplease::{
    ServerConfig, handle_request,
    types::{Implementation, JsonRpcRequest, RequestContext},
};
use serde_json::json;

pub struct State {
    pub greeting: String,
}

mcplease::tools!(State, (Echo, echo, "echo"), (Whoami, whoami, "whoami"));

fn config() -> ServerConfig {
    ServerConfig::new(Implementation::new("test-server", "1.2.3"))
        .with_instructions("say things back")
}

fn call(method: &str, params: serde_json::Value) -> serde_json::Value {
    let request: JsonRpcRequest = serde_json::from_value(
        json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
    )
    .unwrap();
    let mut state = State {
        greeting: "hello".into(),
    };
    let response = handle_request::<Tools, State>(request, &mut state, &config());
    serde_json::to_value(response).unwrap()
}

#[test]
fn tools_list_advertises_the_macro_generated_tool() {
    let response = call("tools/list", json!({}));
    let tools = &response["result"]["tools"];
    assert_eq!(tools.as_array().unwrap().len(), 2);
    assert_eq!(tools[0]["name"], "Echo");
    assert_eq!(
        tools[0]["inputSchema"]["properties"]["message"]["type"],
        "string"
    );
}

#[test]
fn tools_call_dispatches_through_the_generated_enum() {
    let response = call(
        "tools/call",
        json!({"name": "echo", "arguments": {"message": "world"}}),
    );
    assert_eq!(response["result"]["content"][0]["text"], "hello, world");
    assert!(response["result"].get("isError").is_none());
}

/// `params.arguments` is optional in the schema: calling a tool that takes no
/// arguments without the field must work, and absent means the same as `{}`.
#[test]
fn tools_call_without_arguments_dispatches_a_zero_argument_tool() {
    let response = call("tools/call", json!({"name": "whoami"}));
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response["result"]["content"][0]["text"],
        "anonymous on unstated, elicitation: false"
    );
}

#[test]
fn a_tool_failure_is_an_is_error_result_not_a_protocol_error() {
    let response = call(
        "tools/call",
        json!({"name": "echo", "arguments": {"message": ""}}),
    );
    // The model has to be able to see this and self-correct, so it is a
    // successful JSON-RPC response carrying isError.
    assert!(response.get("error").is_none());
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(response["result"]["content"][0]["text"], "nothing to echo");
}

#[test]
fn an_unknown_tool_is_a_protocol_error() {
    let response = call("tools/call", json!({"name": "nope", "arguments": {}}));
    assert_eq!(response["error"]["code"], -32602);
}

#[test]
fn every_result_carries_server_info_meta() {
    for method in ["tools/list", "initialize", "server/discover"] {
        let response = call(method, json!({}));
        let server_info = &response["result"]["_meta"]["io.modelcontextprotocol/serverInfo"];
        assert_eq!(server_info["name"], "test-server", "{method}");
        assert_eq!(server_info["version"], "1.2.3", "{method}");
    }
}

#[test]
fn initialize_echoes_a_supported_version_and_falls_back_otherwise() {
    let params = |version| {
        json!({
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "test-client", "version": "0.1.0"},
        })
    };

    let response = call("initialize", params("2025-06-18"));
    assert_eq!(response["result"]["protocolVersion"], "2025-06-18");

    let response = call("initialize", params("1999-01-01"));
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
}

#[test]
fn version_negotiation_does_not_depend_on_the_rest_of_the_params() {
    // Negotiation is exactly where the two sides have not yet agreed on the
    // message shape, so an absent or unparseable sibling field must not cost
    // the client its requested version.
    for params in [
        json!({"protocolVersion": "2025-06-18"}),
        json!({"protocolVersion": "2025-06-18", "clientInfo": "not an object"}),
        json!({"protocolVersion": "2025-06-18", "somethingFromTheFuture": {"a": 1}}),
    ] {
        let response = call("initialize", params.clone());
        assert_eq!(
            response["result"]["protocolVersion"], "2025-06-18",
            "{params}"
        );
    }
}

#[test]
fn discover_answers_statelessly_with_versions_and_cache_hints() {
    let response = call("server/discover", json!({}));
    let result = &response["result"];
    assert!(
        result["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2026-07-28"))
    );
    assert_eq!(result["instructions"], "say things back");
    assert_eq!(result["ttlMs"], 60 * 60 * 1000);
    assert_eq!(result["cacheScope"], "private");
}

#[test]
fn ping_and_unknown_methods() {
    assert_eq!(call("ping", json!({}))["result"], json!({}));
    assert_eq!(call("nope", json!({}))["error"]["code"], -32601);
}

/// The two halves against each other, with no transport in between.
///
/// `handle_request` and `client::ClientProtocol` are both I/O-free, so a
/// "connection" is a function call: hand the client's request to the server,
/// hand the server's response back. Everything a real transport adds — framing,
/// sockets, subprocesses — is absent, and everything the protocol decides is
/// present.
#[cfg(feature = "client")]
mod loopback {
    use super::{State, Tools, config};
    use mcplease::{
        client::{ClientProtocol, Negotiated, Negotiation, Reaction, Step},
        handle_request,
        types::{
            CallToolResult, Implementation, JsonRpcError, JsonRpcMessage, JsonRpcRequest,
            JsonRpcResponse, ListToolsResult, RequestId, ToolCallOutcome, error_codes,
        },
    };
    use serde_json::{Value, json};

    /// One round trip through the server. The `Reaction` detour is what a real
    /// transport does with every frame it reads.
    fn round_trip(
        protocol: &mut ClientProtocol,
        request: JsonRpcRequest,
    ) -> Result<Value, JsonRpcError> {
        let id = request.id.clone();
        let mut state = State {
            greeting: "hello".into(),
        };
        let response = handle_request::<Tools, State>(request, &mut state, &config());
        match protocol.on_message(JsonRpcMessage::Response(response)) {
            Reaction::Result { id: got, result } => {
                assert_eq!(got, id, "a response arrived for a request we did not send");
                result
            }
            other => panic!("expected a result, got {other:?}"),
        }
    }

    fn client() -> ClientProtocol {
        ClientProtocol::new(Implementation::new("test-client", "9.9.9"))
    }

    fn negotiate(protocol: &mut ClientProtocol) -> Negotiated {
        let (mut negotiation, mut request) = Negotiation::start(protocol);
        loop {
            let result = round_trip(protocol, request);
            match negotiation.on_result(protocol, result).unwrap() {
                Step::Send(next) => request = next,
                Step::Done(negotiated) => return *negotiated,
            }
        }
    }

    #[test]
    fn negotiation_settles_stateless_against_a_server_that_answers_discover() {
        let mut protocol = client();
        let negotiated = negotiate(&mut protocol);

        // This server answers `server/discover`, so the handshake is never
        // reached and there is no session to confirm.
        assert!(!negotiated.stateful);
        assert!(negotiated.initialized_notification().is_none());
        assert_eq!(negotiated.protocol_version, "2026-07-28");
        assert_eq!(protocol.protocol_version(), Some("2026-07-28"));
        assert_eq!(negotiated.instructions.as_deref(), Some("say things back"));
        assert_eq!(negotiated.tools_ttl_ms, Some(60 * 60 * 1000));
        // `server/discover` carries no identity of its own; it comes from the
        // `_meta` the server stamps into every result.
        let server_info = negotiated.server_info.expect("serverInfo in _meta");
        assert_eq!(server_info.name, "test-server");
    }

    #[test]
    fn negotiation_falls_back_to_the_handshake_when_discover_is_unknown() {
        // A server on an earlier revision: `server/discover` is simply a
        // method it does not have, which is the spec's compatibility signal.
        let mut protocol = client();
        let (mut negotiation, request) = Negotiation::start(&mut protocol);
        assert_eq!(request.method, "server/discover");

        let declined = Err(JsonRpcError::method_not_found("server/discover"));
        let Step::Send(request) = negotiation.on_result(&mut protocol, declined).unwrap() else {
            panic!("expected the fallback to initialize");
        };
        assert_eq!(request.method, "initialize");
        // `initialize` states version, identity, and capabilities as ordinary
        // params — stamping the same information into `_meta` as well would be
        // redundant, and an older server reads only the params.
        let params = request.params.clone().unwrap();
        assert_eq!(params["protocolVersion"], "2025-11-25");
        assert_eq!(params["clientInfo"]["name"], "test-client");
        assert!(params.get("_meta").is_none());

        let result = round_trip(&mut protocol, request);
        let Step::Done(negotiated) = negotiation.on_result(&mut protocol, result).unwrap() else {
            panic!("expected negotiation to settle");
        };
        assert!(negotiated.stateful);
        assert_eq!(negotiated.protocol_version, "2025-11-25");
        assert_eq!(
            negotiated.initialized_notification().unwrap().method,
            "notifications/initialized"
        );
        assert_eq!(negotiated.server_info.unwrap().name, "test-server");
    }

    #[test]
    fn a_client_stamp_is_what_the_tool_reads_back_out_of_its_context() {
        // The whole point of the `_meta` half: what the client declares on a
        // request is what `RequestContext` hands the tool on the other side.
        let mut protocol = client();
        negotiate(&mut protocol);

        let request = protocol.tools_call("whoami", &json!({}));
        let result = round_trip(&mut protocol, request).unwrap();
        let ToolCallOutcome::Complete(result) = serde_json::from_value(result).unwrap() else {
            panic!("expected a complete result");
        };
        let CallToolResult { content, .. } = result;
        let text = serde_json::to_value(&content[0]).unwrap();
        assert_eq!(
            text["text"],
            "test-client 9.9.9 on 2026-07-28, elicitation: false"
        );
    }

    #[test]
    fn a_full_listing_and_call_drive_through_the_client_alone() {
        let mut protocol = client();
        negotiate(&mut protocol);

        let mut names = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let request = protocol.tools_list(cursor.as_deref());
            let result = round_trip(&mut protocol, request).unwrap();
            let page: ListToolsResult = serde_json::from_value(result).unwrap();
            names.extend(page.tools.into_iter().map(|tool| tool.name));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(names, ["Echo", "Whoami"]);

        let request = protocol.tools_call("echo", &json!({ "message": "world" }));
        let result = round_trip(&mut protocol, request).unwrap();
        let ToolCallOutcome::Complete(result) = serde_json::from_value(result).unwrap() else {
            panic!("expected a complete result");
        };
        assert_eq!(
            serde_json::to_value(&result.content[0]).unwrap()["text"],
            "hello, world"
        );

        // A tool failure is a successful response carrying isError, and the
        // client must not mistake it for a protocol error.
        let request = protocol.tools_call("echo", &json!({ "message": "" }));
        let result = round_trip(&mut protocol, request).unwrap();
        let ToolCallOutcome::Complete(result) = serde_json::from_value(result).unwrap() else {
            panic!("expected a complete result");
        };
        assert_eq!(result.is_error, Some(true));

        // An unknown tool *is* a protocol error.
        let request = protocol.tools_call("nope", &json!({}));
        let error = round_trip(&mut protocol, request).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_PARAMS);
    }

    #[test]
    fn the_server_can_ping_a_client_that_declared_nothing_else() {
        let mut protocol = client();

        let ping = JsonRpcRequest::new(RequestId::Integer(7), "ping", None);
        let Reaction::Reply(response) = protocol.on_message(JsonRpcMessage::Request(ping)) else {
            panic!("a ping deserves an answer");
        };
        assert_eq!(response.id, Some(RequestId::Integer(7)));
        assert_eq!(response.into_result().unwrap(), json!({}));

        // Everything else a server can initiate belongs to a capability this
        // client did not declare.
        let elicit = JsonRpcRequest::new(RequestId::Integer(8), "elicitation/create", None);
        let Reaction::Reply(response) = protocol.on_message(JsonRpcMessage::Request(elicit)) else {
            panic!("expected a refusal");
        };
        assert_eq!(
            response.into_result().unwrap_err().code,
            error_codes::METHOD_NOT_FOUND
        );
    }

    #[test]
    fn traffic_that_is_not_ours_to_act_on_is_ignored() {
        let mut protocol = client();
        for message in [
            JsonRpcMessage::Notification(mcplease::types::JsonRpcNotification::new(
                "notifications/tools/list_changed",
                None,
            )),
            // A response the server could not attribute to any request.
            JsonRpcMessage::Response(JsonRpcResponse::error(
                None,
                JsonRpcError::internal("unattributable"),
            )),
        ] {
            assert!(matches!(protocol.on_message(message), Reaction::Ignore));
        }

        // A response to a request this client abandoned is still a `Result` —
        // classifying it is the protocol's job, dropping it is the
        // transport's, which knows what it still has outstanding.
        let stale = JsonRpcResponse::success(RequestId::Integer(41), json!({}));
        assert!(matches!(
            protocol.on_message(JsonRpcMessage::Response(stale)),
            Reaction::Result {
                id: RequestId::Integer(41),
                ..
            }
        ));
    }
}

#[test]
fn the_command_line_path_renders_text_without_a_client() {
    use mcplease::traits::Dispatch;
    let mut state = State {
        greeting: "hi".into(),
    };
    let tool: Tools = serde_json::from_value(json!({
        "name": "echo", "arguments": {"message": "there"}
    }))
    .unwrap();
    let text = tool
        .call_to_text(&mut state, &RequestContext::default())
        .unwrap();
    assert_eq!(text, "hi, there");
}
