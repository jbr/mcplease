//! A `Resources` provider installed on the config, served by `handle_request`
//! beside the tools.

use super::{State, Tools, config};
use mcplease::{Resources, ServerConfig, handle_request, types::*};
use serde_json::{Value, json};

struct Catalog;

impl Resources<State> for Catalog {
    async fn list(
        &self,
        state: State,
        params: PaginatedRequestParams,
        context: RequestContext,
    ) -> Result<ListResourcesResult, JsonRpcError> {
        assert!(!context.is_cancelled());
        if params.cursor.as_deref() == Some("context") {
            assert_eq!(context.protocol_version(), Some("2026-07-28"));
        }
        Ok(ListResourcesResult {
            resources: vec![Resource {
                name: state.greeting,
                uri: "efference://models".into(),
                ..Default::default()
            }],
            next_cursor: params.cursor,
            ttl_ms: Some(500),
            cache_scope: Some(CacheScope::Public),
            meta: Some(json!({"provider": true}).as_object().unwrap().clone()),
            ..Default::default()
        })
    }

    async fn list_templates(
        &self,
        _: State,
        _: PaginatedRequestParams,
        _: RequestContext,
    ) -> Result<ListResourceTemplatesResult, JsonRpcError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![ResourceTemplate {
                name: "models".into(),
                uri_template: "efference://models{?q,limit,offset}".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
    }

    async fn read(
        &self,
        state: State,
        params: ReadResourceRequestParams,
        context: RequestContext,
    ) -> Result<ReadResourceResult, JsonRpcError> {
        if params.uri == "missing" {
            return Err(JsonRpcError::invalid_params("missing resource"));
        }
        if params.uri == "broken" {
            return Err(JsonRpcError::internal("backend failed"));
        }
        assert!(!context.is_cancelled());
        Ok(ReadResourceResult {
            contents: vec![
                ResourceContents::text(params.uri, state.greeting).with_mime_type("text/plain"),
                BlobResourceContents {
                    uri: "custom:blob".into(),
                    blob: "AA==".into(),
                    meta: Some(json!({"page": 2}).as_object().unwrap().clone()),
                    ..Default::default()
                }
                .into(),
            ],
            ..Default::default()
        })
    }
}

fn request(method: &str, params: Value) -> JsonRpcRequest {
    serde_json::from_value(json!({"jsonrpc":"2.0", "id":1,"method":method,"params":params}))
        .unwrap()
}

fn dispatch<R: Resources<State>>(config: &ServerConfig<R>, method: &str, params: Value) -> Value {
    let request = request(method, params);
    let context = RequestContext::from_request(&request);
    let state = State {
        greeting: "cached models".into(),
    };
    let response = pollster::block_on(handle_request::<Tools, State>(
        request, state, context, config,
    ))
    .expect("nothing here is cancellable");
    serde_json::to_value(response).unwrap()
}

fn call(method: &str, params: Value) -> Value {
    dispatch(&config().with_resources(Catalog), method, params)
}

#[test]
fn installing_a_provider_advertises_resources_beside_tools() {
    for method in ["initialize", "server/discover"] {
        let response = call(method, json!({}));
        let capabilities = &response["result"]["capabilities"];
        assert_eq!(capabilities["resources"], json!({}));
        assert!(capabilities.get("tools").is_some());
    }
    assert_eq!(
        call("tools/list", json!({}))["result"]["tools"][0]["name"],
        "Echo"
    );
    let echo = call(
        "tools/call",
        json!({"name": "echo", "arguments": {"message": "world"}}),
    );
    assert_eq!(echo["result"]["content"][0]["text"], "cached models, world");
    for method in [
        "resources/subscribe",
        "subscriptions/listen",
        "completion/complete",
    ] {
        assert_eq!(call(method, json!({}))["error"]["code"], -32601);
    }
}

#[test]
fn discovery_preserves_pagination_cache_and_provider_metadata() {
    let response = call("resources/list", json!({"cursor":"opaque/page"}));
    let result = &response["result"];
    assert_eq!(result["resources"][0]["name"], "cached models");
    assert_eq!(result["nextCursor"], "opaque/page");
    assert_eq!(result["ttlMs"], 500);
    assert_eq!(result["cacheScope"], "public");
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["_meta"]["provider"], true);
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "test-server"
    );
    let templates = call("resources/templates/list", json!({}));
    assert_eq!(
        templates["result"]["resourceTemplates"][0]["uriTemplate"],
        "efference://models{?q,limit,offset}"
    );
    assert_eq!(templates["result"]["ttlMs"], 0);
    assert_eq!(templates["result"]["cacheScope"], "private");
}

#[test]
fn reads_preserve_uri_and_multiple_contents() {
    let uri = "efference://models?q=a%2fb+%20&limit=3&offset=8";
    let result = call("resources/read", json!({"uri":uri}));
    assert_eq!(result["result"]["contents"][0]["uri"], uri);
    assert_eq!(result["result"]["contents"][0]["text"], "cached models");
    assert_eq!(result["result"]["contents"][0]["mimeType"], "text/plain");
    assert_eq!(result["result"]["contents"][1]["blob"], "AA==");
    assert_eq!(result["result"]["contents"][1]["_meta"]["page"], 2);
    assert!(result["result"].get("isError").is_none());
}

#[test]
fn errors_are_protocol_errors_and_params_are_validated() {
    for params in [json!({}), json!({"uri":7}), Value::Null] {
        assert_eq!(call("resources/read", params)["error"]["code"], -32602);
    }
    assert_eq!(
        call("resources/list", json!({"cursor":7}))["error"]["code"],
        -32602
    );
    assert_eq!(
        call("resources/read", json!({"uri":"missing"}))["error"]["code"],
        -32602
    );
    assert_eq!(
        call("resources/read", json!({"uri":"broken"}))["error"]["code"],
        -32603
    );
}

#[test]
fn without_a_provider_resources_are_neither_advertised_nor_served() {
    let response = dispatch(&config(), "server/discover", json!({}));
    assert!(
        response["result"]["capabilities"]
            .get("resources")
            .is_none()
    );
    for method in [
        "resources/list",
        "resources/templates/list",
        "resources/read",
    ] {
        let response = dispatch(&config(), method, json!({"uri": "efference://models"}));
        assert_eq!(response["error"]["code"], -32601, "{method}");
    }
}

#[test]
fn discovery_receives_current_request_context() {
    call(
        "resources/list",
        json!({"cursor":"context", "_meta": {"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}),
    );
}
