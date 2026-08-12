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

mcplease::tools!(State, (Echo, echo, "echo"));

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
    assert_eq!(tools.as_array().unwrap().len(), 1);
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
