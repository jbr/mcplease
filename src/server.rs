//! Transport-agnostic server dispatch.
//!
//! [`handle_request`] maps a decoded [`JsonRpcRequest`] to a [`JsonRpcResponse`]
//! and performs no I/O of its own, so a transport is only responsible for
//! framing: read a message, hand any request here, write the response back. The
//! stdio loop in [`crate::stdio`] is one such transport, and is thin because of
//! it.

use crate::{
    traits::{AsToolsList, Dispatch},
    types::{
        CacheScope, CallToolResult, DiscoverResult, Implementation, InitializeResult, JsonRpcError,
        JsonRpcRequest, JsonRpcResponse, LATEST_HANDSHAKE_PROTOCOL_VERSION, ListToolsResult,
        RequestContext, ResultType, SUPPORTED_PROTOCOL_VERSIONS, ServerCapabilities, meta_keys,
    },
};
use serde_json::{Map, Value, json};
use std::fmt::Debug;

/// How long a client may consider a `tools/list` result fresh.
///
/// The tool list is built from macro expansion and cannot change while the
/// process runs, so it is only ever stale across binary versions — and the
/// `serverInfo` this server stamps into every result carries the version, so a
/// client can detect that directly. An hour is therefore comfortable; the
/// spec's own worked example uses five minutes, calibrated for servers whose
/// lists genuinely change.
pub const DEFAULT_TOOLS_TTL_MS: u64 = 60 * 60 * 1000;

/// Everything the serve loop needs beyond the tools themselves.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    server_info: Implementation,
    instructions: Option<&'static str>,
    tools_ttl_ms: u64,
    tools_cache_scope: CacheScope,
}

impl ServerConfig {
    pub fn new(server_info: Implementation) -> Self {
        Self {
            server_info,
            instructions: None,
            tools_ttl_ms: DEFAULT_TOOLS_TTL_MS,
            tools_cache_scope: CacheScope::Private,
        }
    }

    /// Guidance handed to the model about how to use this server.
    pub fn with_instructions(mut self, instructions: &'static str) -> Self {
        self.instructions = Some(instructions);
        self
    }

    /// Override the `tools/list` freshness hint. See [`DEFAULT_TOOLS_TTL_MS`].
    pub fn with_tools_ttl_ms(mut self, tools_ttl_ms: u64) -> Self {
        self.tools_ttl_ms = tools_ttl_ms;
        self
    }

    /// Allow shared caches to serve this server's tool list across
    /// authorization contexts.
    ///
    /// Defaults to [`CacheScope::Private`]. `Public` is the more accurate
    /// answer for a list that is identical for every user — which a
    /// macro-generated list is — but it permits intermediaries to share the
    /// response between callers, so it is opt-in: a server that later filters
    /// tools per-user would otherwise leak the list across tenants by default.
    pub fn with_tools_cache_scope(mut self, tools_cache_scope: CacheScope) -> Self {
        self.tools_cache_scope = tools_cache_scope;
        self
    }

    /// `_meta` identifying this server, which the spec asks be included in
    /// every result.
    fn result_meta(&self) -> Option<Map<String, Value>> {
        Some(
            json!({ meta_keys::SERVER_INFO: self.server_info })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        )
    }
}

/// Answer one request. Dispatch is stateless: every supported revision's
/// entry points are answerable at any time, so both the `initialize`
/// handshake (revisions through 2025-11-25) and the stateless
/// `server/discover` flow (2026-07-28) work against the same loop.
pub fn handle_request<Tools: Debug + AsToolsList + Dispatch<State>, State>(
    request: JsonRpcRequest,
    state: &mut State,
    config: &ServerConfig,
) -> JsonRpcResponse {
    let JsonRpcRequest {
        id, method, params, ..
    } = request;
    let instructions = config.instructions;
    let server_info = &config.server_info;
    // Rebuilt per request: `2026-07-28` declares capabilities per-request and
    // forbids inferring them from earlier ones.
    let context = RequestContext::from_params(params.as_ref());
    match method.as_str() {
        "initialize" => {
            // Echo a supported requested version; otherwise answer with the
            // newest handshake revision and let the client decide.
            //
            // Read straight off the params rather than through
            // `InitializeRequestParams`: negotiating the version is precisely
            // the point at which the two sides have not yet agreed on what the
            // rest of the message looks like, so failing to parse a sibling
            // field must not cost the client its requested version.
            let requested = params
                .as_ref()
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str);
            let protocol_version = match requested {
                Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v.to_string(),
                _ => LATEST_HANDSHAKE_PROTOCOL_VERSION.to_string(),
            };
            JsonRpcResponse::success(
                id,
                InitializeResult {
                    protocol_version,
                    capabilities: ServerCapabilities::tools_only(),
                    server_info: server_info.clone(),
                    instructions: instructions.map(String::from),
                    meta: config.result_meta(),
                },
            )
        }
        "server/discover" => JsonRpcResponse::success(
            id,
            DiscoverResult {
                supported_versions: SUPPORTED_PROTOCOL_VERSIONS
                    .iter()
                    .map(|v| v.to_string())
                    .collect(),
                capabilities: ServerCapabilities::tools_only(),
                instructions: instructions.map(String::from),
                ttl_ms: Some(config.tools_ttl_ms),
                cache_scope: Some(config.tools_cache_scope),
                result_type: Some(ResultType::Complete),
                meta: config.result_meta(),
            },
        ),
        "ping" => JsonRpcResponse::success(id, json!({})),
        "tools/list" => JsonRpcResponse::success(
            id,
            ListToolsResult {
                tools: Tools::tools_list(),
                ttl_ms: Some(config.tools_ttl_ms),
                cache_scope: Some(config.tools_cache_scope),
                result_type: Some(ResultType::Complete),
                meta: config.result_meta(),
                ..ListToolsResult::default()
            },
        ),
        "tools/call" => {
            match serde_json::from_value::<Tools>(params.unwrap_or(serde_json::Value::Null)) {
                Ok(tool) => {
                    log::info!("{tool:?}");
                    match tool.call(state, &context) {
                        Ok(result) => {
                            log::debug!("{result:?}");
                            JsonRpcResponse::success(
                                id,
                                CallToolResult {
                                    meta: config.result_meta(),
                                    ..result
                                },
                            )
                        }
                        // A failure of the tool itself is an is_error result,
                        // not a protocol error, so the model sees it.
                        Err(e) => {
                            log::error!("{e}");
                            JsonRpcResponse::success(
                                id,
                                CallToolResult {
                                    meta: config.result_meta(),
                                    ..CallToolResult::error(e.to_string())
                                },
                            )
                        }
                    }
                }
                Err(e) => {
                    log::error!("{e}");
                    JsonRpcResponse::error(id, JsonRpcError::invalid_params(e.to_string()))
                }
            }
        }
        _ => JsonRpcResponse::error(id, JsonRpcError::method_not_found(&method)),
    }
}
