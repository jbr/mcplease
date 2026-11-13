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
        let (name, uri, next_cursor) = match params.cursor.as_deref() {
            None => (state.greeting, "efference://models", Some("page 2".into())),
            Some("page 2") => ("second page".into(), "efference://second", None),
            Some(cursor) => {
                return Err(JsonRpcError::invalid_params(format!(
                    "unknown cursor {cursor:?}"
                )));
            }
        };
        Ok(ListResourcesResult {
            resources: vec![Resource {
                name,
                uri: uri.into(),
                ..Default::default()
            }],
            next_cursor,
            ttl_ms: Some(500),
            cache_scope: Some(CacheScope::Public),
            meta: Some(
                json!({"provider": true, "protocolVersion": context.protocol_version()})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
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
            return Err(JsonRpcError::resource_not_found(&params.uri));
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

fn respond<R: Resources<State>>(
    config: &ServerConfig<R>,
    request: JsonRpcRequest,
) -> JsonRpcResponse {
    let context = RequestContext::from_request(&request);
    let state = State {
        greeting: "cached models".into(),
    };
    pollster::block_on(handle_request::<Tools, State>(
        request, state, context, config,
    ))
    .expect("nothing here is cancellable")
}

fn dispatch<R: Resources<State>>(config: &ServerConfig<R>, method: &str, params: Value) -> Value {
    serde_json::to_value(respond(config, request(method, params))).unwrap()
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
    let response = call("resources/list", json!({}));
    let result = &response["result"];
    assert_eq!(result["resources"][0]["name"], "cached models");
    assert_eq!(result["nextCursor"], "page 2");
    let second = call("resources/list", json!({"cursor": "page 2"}));
    assert_eq!(second["result"]["resources"][0]["name"], "second page");
    assert!(second["result"].get("nextCursor").is_none());
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
    for cursor in [json!(7), json!("opaque")] {
        assert_eq!(
            call("resources/list", json!({"cursor": cursor}))["error"]["code"],
            -32602
        );
    }
    let missing = call("resources/read", json!({"uri":"missing"}));
    assert_eq!(missing["error"]["code"], -32602);
    assert_eq!(missing["error"]["data"]["uri"], "missing");
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
    let response = call(
        "resources/list",
        json!({"_meta": {"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}),
    );
    assert_eq!(response["result"]["_meta"]["protocolVersion"], "2026-07-28");
}

/// The client half against the server half: what `ClientProtocol` builds is
/// what the provider serves, and what comes back decodes as the client's types.
#[cfg(feature = "client")]
mod client {
    use super::*;
    use mcplease::client::{ClientProtocol, Paginated, Reaction};

    fn round_trip(
        protocol: &mut ClientProtocol,
        request: JsonRpcRequest,
    ) -> Result<Value, JsonRpcError> {
        let id = request.id.clone();
        let response = respond(&config().with_resources(Catalog), request);
        match protocol.on_message(JsonRpcMessage::Response(response)) {
            Reaction::Result { id: got, result } => {
                assert_eq!(got, id);
                result
            }
            other => panic!("expected a result, got {other:?}"),
        }
    }

    /// The loop a transport writes once for every listing.
    fn drain<P: Paginated + serde::de::DeserializeOwned>(
        protocol: &mut ClientProtocol,
    ) -> Vec<P::Item> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let request = protocol.page::<P>(cursor.as_deref());
            let page: P = serde_json::from_value(round_trip(protocol, request).unwrap()).unwrap();
            cursor = page.next_cursor().map(String::from);
            items.extend(page.into_items());
            if cursor.is_none() {
                return items;
            }
        }
    }

    fn client() -> ClientProtocol {
        ClientProtocol::new(Implementation::new("test-client", "9.9.9"))
    }

    #[test]
    fn a_client_drains_every_listing_with_one_loop() {
        let mut protocol = client();
        let resources = drain::<ListResourcesResult>(&mut protocol);
        let names: Vec<_> = resources.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["cached models", "second page"]);

        let templates = drain::<ListResourceTemplatesResult>(&mut protocol);
        assert_eq!(
            templates[0].uri_template,
            "efference://models{?q,limit,offset}"
        );

        let tools = drain::<ListToolsResult>(&mut protocol);
        assert_eq!(tools.len(), 2);

        // The named builders are the same requests.
        assert_eq!(
            protocol.resources_list(Some("page 2")).method,
            "resources/list"
        );
        assert_eq!(
            protocol.resources_templates_list(None).method,
            "resources/templates/list"
        );
    }

    #[test]
    fn a_client_reads_a_listed_resource_and_decodes_its_contents() {
        let mut protocol = client();
        let uri = &drain::<ListResourcesResult>(&mut protocol)[0].uri;
        let request = protocol.resources_read(uri);
        let result = round_trip(&mut protocol, request).unwrap();
        let Outcome::Complete(read) =
            serde_json::from_value::<Outcome<ReadResourceResult>>(result).unwrap()
        else {
            panic!("expected a complete read");
        };
        let ResourceContents::Text(text) = &read.contents[0] else {
            panic!("expected text first");
        };
        assert_eq!(text.uri, *uri);
        assert_eq!(text.text, "cached models");
        assert!(matches!(read.contents[1], ResourceContents::Blob(_)));
        assert_eq!(read.ttl_ms, Some(0));
        assert_eq!(read.cache_scope, Some(CacheScope::Private));
    }

    #[test]
    fn a_missing_resource_reaches_the_client_as_invalid_params() {
        let mut protocol = client();
        let request = protocol.resources_read("missing");
        let error = round_trip(&mut protocol, request).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_PARAMS);
        assert_eq!(error.data.unwrap()["uri"], "missing");
    }

    #[test]
    fn a_client_stamp_reaches_the_provider() {
        let mut protocol = client();
        let request = protocol.resources_list(None);
        let result = round_trip(&mut protocol, request).unwrap();
        assert_eq!(
            result["_meta"]["protocolVersion"],
            mcplease::types::LATEST_PROTOCOL_VERSION
        );
    }
}
