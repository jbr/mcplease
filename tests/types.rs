use mcplease::types::*;
use serde_json::{Value, json};

fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned>(value: Value) -> Value {
    let typed: T = serde_json::from_value(value).unwrap();
    serde_json::to_value(&typed).unwrap()
}

#[test]
fn message_discrimination() {
    let request: JsonRpcMessage =
        serde_json::from_value(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
    assert!(matches!(request, JsonRpcMessage::Request(_)));

    let notification: JsonRpcMessage =
        serde_json::from_value(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .unwrap();
    assert!(matches!(notification, JsonRpcMessage::Notification(_)));

    let response: JsonRpcMessage =
        serde_json::from_value(json!({"jsonrpc": "2.0", "id": "a", "result": {}})).unwrap();
    assert!(matches!(response, JsonRpcMessage::Response(_)));

    let error: JsonRpcMessage = serde_json::from_value(
        json!({"jsonrpc": "2.0", "id": 2, "error": {"code": -32601, "message": "nope"}}),
    )
    .unwrap();
    let JsonRpcMessage::Response(error) = error else {
        panic!("expected response");
    };
    assert_eq!(
        error.into_result().unwrap_err().code,
        error_codes::METHOD_NOT_FOUND
    );
}

#[test]
fn tool_wire_shape_is_camel_case_and_lossless_for_arbitrary_schemas() {
    let wire = json!({
        "name": "search",
        "description": "Search things",
        "inputSchema": {
            "type": "object",
            "properties": { "q": { "type": "string" } },
            "required": ["q"],
            "if": { "properties": { "q": { "const": "x" } } },
            "$defs": { "anything": { "anyOf": [{ "type": "null" }] } }
        },
        "outputSchema": { "type": "object" },
        "annotations": { "readOnlyHint": true }
    });
    assert_eq!(roundtrip::<Tool>(wire.clone()), wire);
}

#[test]
fn content_block_tags() {
    let text = json!({"type": "text", "text": "hi"});
    assert_eq!(roundtrip::<ContentBlock>(text.clone()), text);

    let image = json!({"type": "image", "data": "aGk=", "mimeType": "image/png"});
    assert_eq!(roundtrip::<ContentBlock>(image.clone()), image);

    let link = json!({"type": "resource_link", "uri": "file:///x", "name": "x"});
    assert_eq!(roundtrip::<ContentBlock>(link.clone()), link);

    let embedded = json!({
        "type": "resource",
        "resource": {"uri": "file:///x", "text": "content", "mimeType": "text/plain"}
    });
    assert_eq!(roundtrip::<ContentBlock>(embedded.clone()), embedded);
}

#[test]
fn call_tool_result_constructors() {
    let ok = serde_json::to_value(CallToolResult::text("done")).unwrap();
    assert_eq!(
        ok,
        json!({
            "content": [{"type": "text", "text": "done"}],
            "resultType": "complete"
        })
    );

    let failed = serde_json::to_value(CallToolResult::error("broke")).unwrap();
    assert_eq!(failed["isError"], json!(true));
}

#[test]
fn initialize_result_parses_from_every_handshake_revision() {
    // 2024-11-05 era: no instructions, sparse capabilities.
    let old: InitializeResult = serde_json::from_value(json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "s", "version": "1"}
    }))
    .unwrap();
    assert_eq!(old.protocol_version, "2024-11-05");

    // 2025-11-25 era: title/icons/websiteUrl on serverInfo.
    let new: InitializeResult = serde_json::from_value(json!({
        "protocolVersion": "2025-11-25",
        "capabilities": {"tools": {"listChanged": true}, "extensions": {"io.modelcontextprotocol/tasks": {}}},
        "serverInfo": {"name": "s", "version": "1", "title": "Server", "websiteUrl": "https://example.com"},
        "instructions": "be nice"
    }))
    .unwrap();
    assert_eq!(new.capabilities.tools.unwrap().list_changed, Some(true));
}

#[test]
fn list_tools_result_tolerates_missing_2026_fields() {
    // A pre-2026-07-28 server omits ttlMs/cacheScope/resultType.
    let result: ListToolsResult = serde_json::from_value(json!({
        "tools": [{"name": "t", "inputSchema": {"type": "object"}}]
    }))
    .unwrap();
    assert_eq!(result.tools.len(), 1);
    assert_eq!(result.result_type, None); // spec: absent means "complete"
}

#[test]
fn result_type_round_trips_and_preserves_unknown_values() {
    assert_eq!(
        serde_json::from_value::<ResultType>(json!("complete")).unwrap(),
        ResultType::Complete
    );
    assert_eq!(
        serde_json::from_value::<ResultType>(json!("input_required")).unwrap(),
        ResultType::InputRequired
    );

    // A revision after 2026-07-28 may add result types; keep the value rather
    // than failing to parse the response.
    let future: ResultType = serde_json::from_value(json!("something_new")).unwrap();
    assert_eq!(future, ResultType::Other("something_new".into()));
    assert_eq!(
        serde_json::to_value(&future).unwrap(),
        json!("something_new")
    );
}

#[test]
fn tool_call_outcome_discriminates_on_result_type() {
    // The failure this prevents: an input_required result read as a
    // successful call that happens to have returned no content.
    let complete: ToolCallOutcome = serde_json::from_value(json!({
        "resultType": "complete",
        "content": [{"type": "text", "text": "hi"}]
    }))
    .unwrap();
    assert!(matches!(complete, ToolCallOutcome::Complete(_)));

    // requestState with no inputRequests: the load-shedding case, which needs
    // no declared client capability, so any client can receive one.
    let shed: ToolCallOutcome = serde_json::from_value(json!({
        "resultType": "input_required",
        "requestState": "opaque-blob"
    }))
    .unwrap();
    let ToolCallOutcome::InputRequired(shed) = shed else {
        panic!("expected input_required");
    };
    assert_eq!(shed.request_state.as_deref(), Some("opaque-blob"));
    assert!(shed.input_requests.is_none());

    let elicit: ToolCallOutcome = serde_json::from_value(json!({
        "resultType": "input_required",
        "requestState": "state",
        "inputRequests": {
            "confirm": {
                "method": "elicitation/create",
                "params": {"message": "sure?", "requestedSchema": {"type": "object", "properties": {}}}
            }
        }
    }))
    .unwrap();
    let ToolCallOutcome::InputRequired(elicit) = elicit else {
        panic!("expected input_required");
    };
    assert!(elicit.input_requests.unwrap().contains_key("confirm"));

    // A server on an earlier revision omits resultType; the spec directs
    // clients to treat that as complete.
    let legacy: ToolCallOutcome = serde_json::from_value(json!({
        "content": [{"type": "text", "text": "hi"}]
    }))
    .unwrap();
    assert!(matches!(legacy, ToolCallOutcome::Complete(_)));
}

#[test]
fn request_context_reads_per_request_meta() {
    let context = RequestContext::from_params(Some(&json!({
        "name": "some_tool",
        "arguments": {},
        "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {"name": "harness", "version": "9"},
            "io.modelcontextprotocol/clientCapabilities": {"elicitation": {"form": {}}}
        }
    })));

    assert_eq!(context.protocol_version.as_deref(), Some("2026-07-28"));
    assert_eq!(context.client_info.as_ref().unwrap().name, "harness");
    assert!(context.supports_elicitation());

    // A request from an earlier revision carries none of this.
    let legacy = RequestContext::from_params(Some(&json!({"name": "t", "arguments": {}})));
    assert_eq!(legacy.protocol_version, None);
    assert!(!legacy.supports_elicitation());
}
