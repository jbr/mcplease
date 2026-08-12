use mcplease::types::*;
use serde_json::{json, Value};

fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned>(value: Value) -> Value {
    let typed: T = serde_json::from_value(value).unwrap();
    serde_json::to_value(&typed).unwrap()
}

#[test]
fn message_discrimination() {
    let request: JsonRpcMessage =
        serde_json::from_value(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .unwrap();
    assert!(matches!(request, JsonRpcMessage::Request(_)));

    let notification: JsonRpcMessage = serde_json::from_value(
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
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
