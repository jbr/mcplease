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
        RequestContext, ResultType, SUPPORTED_PROTOCOL_VERSIONS, ServerCapabilities,
        SetLevelRequestParams, Tool, meta_keys,
    },
};
use serde::de::DeserializeOwned;
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

/// How many `tools/call` requests a transport runs at once. See
/// [`ServerConfig::with_max_concurrent_calls`].
pub const DEFAULT_MAX_CONCURRENT_CALLS: usize = 16;

/// Everything the serve loop needs beyond the tools themselves.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    server_info: Implementation,
    instructions: Option<&'static str>,
    tools_ttl_ms: u64,
    tools_cache_scope: CacheScope,
    max_concurrent_calls: usize,
    experimental: Map<String, Value>,
    logging: bool,
}

impl ServerConfig {
    pub fn new(server_info: Implementation) -> Self {
        Self {
            server_info,
            instructions: None,
            tools_ttl_ms: DEFAULT_TOOLS_TTL_MS,
            tools_cache_scope: CacheScope::Private,
            max_concurrent_calls: DEFAULT_MAX_CONCURRENT_CALLS,
            experimental: Map::new(),
            logging: false,
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

    /// How many `tools/call` requests may be running at once. A call that
    /// arrives when this many are already in flight is refused immediately
    /// with [`JsonRpcError::server_busy`] rather than queued, which is what
    /// keeps the bound on work rather than on memory. See
    /// [`DEFAULT_MAX_CONCURRENT_CALLS`].
    ///
    /// `1` restores single-file execution, for a server whose tools form
    /// multi-request state machines that assume it. That is worth treating as
    /// a migration aid rather than a resting place: `2026-07-28` removed
    /// sessions, and a tool sequence that depends on being the only one
    /// running is holding state the protocol expects to travel in requests.
    ///
    /// Only transports read this; [`handle_request`] runs the one call it is
    /// given.
    pub fn with_max_concurrent_calls(mut self, max_concurrent_calls: usize) -> Self {
        self.max_concurrent_calls = max_concurrent_calls.max(1);
        self
    }

    /// How many calls a transport may run at once.
    pub fn max_concurrent_calls(&self) -> usize {
        self.max_concurrent_calls
    }

    /// Advertise a non-standard capability under `capabilities.experimental`,
    /// in both the `initialize` result and `server/discover`. Setting the same
    /// `name` again replaces its value.
    ///
    /// The schema leaves the key space and the value shape to whoever defines
    /// the extension; [`claude_channel::CAPABILITY`] is one such key.
    ///
    /// [`claude_channel::CAPABILITY`]: crate::claude_channel::CAPABILITY
    pub fn with_experimental_capability(mut self, name: impl Into<String>, value: Value) -> Self {
        self.experimental.insert(name.into(), value);
        self
    }

    /// Declare the `logging` capability, so this server may send
    /// `notifications/message`, and answer `logging/setLevel`.
    ///
    /// The level a client sets belongs to its connection, so
    /// [`answer_stateless`] only validates and acknowledges it; a transport
    /// with connection state records it and drops log messages below it. The
    /// stdio transport does, for both a call's notifier and the
    /// [`ConnectionNotifier`](crate::ConnectionNotifier) — whose
    /// [`log`](crate::ConnectionNotifier::log) is the sending helper. Before
    /// any `logging/setLevel`, every level is sent.
    ///
    /// Without this, `logging/setLevel` is an unknown method, and the stdio
    /// transport drops any `notifications/message` it is handed, since the
    /// server never declared it would send them.
    pub fn with_logging(mut self) -> Self {
        self.logging = true;
        self
    }

    /// Whether this server declared the `logging` capability.
    pub fn logging(&self) -> bool {
        self.logging
    }

    /// What this server advertises: tools, plus logging when declared, plus
    /// any experimental entries.
    fn capabilities(&self) -> ServerCapabilities {
        ServerCapabilities {
            experimental: (!self.experimental.is_empty()).then(|| self.experimental.clone()),
            logging: self.logging.then(|| json!({})),
            ..ServerCapabilities::tools_only()
        }
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
///
/// This is [`answer_stateless`] followed, for `tools/call`, by
/// [`answer_tool_call`]. A transport that wants to keep protocol traffic
/// moving while a tool runs calls the two halves itself, from different
/// threads; the stdio transport does exactly that.
///
/// Returns `None` when the call was cancelled: the spec directs a server not
/// to respond to a request the client withdrew. A transport with no way to
/// learn about cancellation — one that never calls
/// [`RequestContext::with_cancellation`] — never sees `None` here.
pub async fn handle_request<Tools, State>(
    request: JsonRpcRequest,
    state: State,
    context: RequestContext,
    config: &ServerConfig,
) -> Option<JsonRpcResponse>
where
    Tools: Debug + AsToolsList + Dispatch<State>,
    State: Clone + Send + Sync + 'static,
{
    match answer_stateless(&request, &Tools::tools_list(), config) {
        Some(response) => Some(response),
        None => answer_tool_call::<Tools, State>(request, state, context, config).await,
    }
}

/// Answer a request that needs no tool state: `initialize`,
/// `server/discover`, `ping`, `tools/list`, `logging/setLevel` when the server
/// declared logging, and any unknown method. Returns `None` for `tools/call`,
/// which is the one request that does.
///
/// `logging/setLevel` is validated and acknowledged here and recorded nowhere:
/// the level belongs to a connection, which this function does not have. The
/// stdio transport records it from a successful answer.
///
/// Takes the advertised tool list rather than a `Tools` type so that a
/// transport can hand this to a thread that never names the tools; the list
/// is fixed for the life of the process, so computing
/// [`AsToolsList::tools_list`] once and passing it here is exact.
pub fn answer_stateless(
    request: &JsonRpcRequest,
    tools: &[Tool],
    config: &ServerConfig,
) -> Option<JsonRpcResponse> {
    let JsonRpcRequest {
        id, method, params, ..
    } = request;
    let id = id.clone();
    let instructions = config.instructions;
    let server_info = &config.server_info;
    let response = match method.as_str() {
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
                    capabilities: config.capabilities(),
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
                capabilities: config.capabilities(),
                instructions: instructions.map(String::from),
                ttl_ms: Some(config.tools_ttl_ms),
                cache_scope: Some(config.tools_cache_scope),
                result_type: Some(ResultType::Complete),
                meta: config.result_meta(),
            },
        ),
        "ping" => JsonRpcResponse::success(id, json!({})),
        "logging/setLevel" if config.logging => {
            match params
                .clone()
                .map(serde_json::from_value::<SetLevelRequestParams>)
            {
                Some(Ok(_)) => JsonRpcResponse::success(id, json!({})),
                Some(Err(e)) => {
                    JsonRpcResponse::error(id, JsonRpcError::invalid_params(e.to_string()))
                }
                None => JsonRpcResponse::error(id, JsonRpcError::invalid_params("missing params")),
            }
        }
        "tools/list" => JsonRpcResponse::success(
            id,
            ListToolsResult {
                tools: tools.to_vec(),
                ttl_ms: Some(config.tools_ttl_ms),
                cache_scope: Some(config.tools_cache_scope),
                result_type: Some(ResultType::Complete),
                meta: config.result_meta(),
                ..ListToolsResult::default()
            },
        ),
        "tools/call" => return None,
        _ => JsonRpcResponse::error(id, JsonRpcError::method_not_found(method)),
    };
    Some(response)
}

/// Answer a `tools/call`: decode the tool from the params, run it against
/// `state`, and wrap what it returns.
///
/// `context` is built by the transport, which is the only layer that can
/// supply the [`Notifier`](crate::types::Notifier) a tool reports progress
/// through and the flag a cancelling client sets — so this function still
/// performs no I/O and names no sink of its own. Build it with
/// [`RequestContext::from_request`] and add what the transport has.
///
/// Returns `None` if the request was cancelled while the tool ran; the spec
/// says not to answer a cancelled request. Whatever the tool returned is
/// discarded in that case, including an error.
///
/// Any request whose method is not `tools/call` is answered with
/// method-not-found; route with [`answer_stateless`] first.
pub async fn answer_tool_call<Tools, State>(
    request: JsonRpcRequest,
    state: State,
    context: RequestContext,
    config: &ServerConfig,
) -> Option<JsonRpcResponse>
where
    Tools: Debug + Dispatch<State> + DeserializeOwned,
    State: Clone + Send + Sync + 'static,
{
    let JsonRpcRequest {
        id, method, params, ..
    } = request;
    if method != "tools/call" {
        return Some(JsonRpcResponse::error(
            id,
            JsonRpcError::method_not_found(&method),
        ));
    }

    // Scoped so that `Tools` itself is never held across the await — the
    // binding would count even after `call` moved it — and only the call's
    // own future, which is declared `Send`, is.
    let call = {
        let tool = match serde_json::from_value::<Tools>(params.unwrap_or(Value::Null)) {
            Ok(tool) => tool,
            Err(e) => {
                log::error!("{e}");
                return Some(JsonRpcResponse::error(
                    id,
                    JsonRpcError::invalid_params(e.to_string()),
                ));
            }
        };
        log::info!("{tool:?}");
        tool.call(state, context.clone())
    };
    let result = match call.await {
        Ok(result) => {
            log::debug!("{result:?}");
            result
        }
        // A failure of the tool itself is an is_error result, not a protocol
        // error, so the model sees it.
        Err(e) => {
            log::error!("{e}");
            CallToolResult::error(e.to_string())
        }
    };
    let response = JsonRpcResponse::success(
        id,
        CallToolResult {
            meta: config.result_meta(),
            ..result
        },
    );

    // Checked after the fact as well as by the tool: cancellation may arrive
    // at any point, including after the work finished, and the spec asks both
    // sides to handle that race by dropping the answer.
    if context.is_cancelled() {
        log::debug!("dropping the response to a cancelled request");
        return None;
    }

    Some(response)
}
