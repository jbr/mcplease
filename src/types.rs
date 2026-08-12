//! Serialization types for the Model Context Protocol.
//!
//! Shapes follow the `2026-07-28` schema revision (reference copies under
//! `spec/`), plus the stateful `initialize` handshake from `2025-11-25` and
//! earlier: `2026-07-28` removed the handshake in favor of stateless
//! per-request `_meta`, but every earlier revision — and most deployed
//! servers — still begins with `initialize`. Fields serialize in the spec's
//! camelCase; optional fields are omitted rather than serialized as null.
//!
//! Fields that `2026-07-28` requires but earlier revisions lack (for
//! example `resultType`, `ttlMs`, `cacheScope`) are `Option` here so the
//! same types parse messages from servers on any revision; the spec directs
//! clients to treat an absent `resultType` as `"complete"`.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;

/// The newest protocol revision these types model.
pub const LATEST_PROTOCOL_VERSION: &str = "2026-07-28";

/// The newest revision that begins with the `initialize` handshake.
/// Revisions after this are stateless and carry version/capability
/// information in per-request `_meta` instead.
pub const LATEST_HANDSHAKE_PROTOCOL_VERSION: &str = "2025-11-25";

/// Every revision the types in this module can represent, newest first.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

// --- JSON-RPC envelope ---

/// A JSON-RPC request id: a string or an integer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    String(String),
    Integer(i64),
}

impl From<i64> for RequestId {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<String> for RequestId {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for RequestId {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}

/// A request that expects a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: Cow<'static, str>,
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: impl Into<RequestId>, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: Cow::Borrowed("2.0"),
            id: id.into(),
            method: method.into(),
            params,
        }
    }
}

/// A one-way message that expects no response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: Cow<'static, str>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: Cow::Borrowed("2.0"),
            method: method.into(),
            params,
        }
    }
}

/// A response to a request: exactly one of `result` or `error` is present.
/// Error responses may omit `id` when the request id could not be read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: RequestId, result: impl Serialize) -> Self {
        Self {
            jsonrpc: Cow::Borrowed("2.0"),
            id: Some(id),
            result: Some(serde_json::to_value(result).unwrap_or(Value::Null)),
            error: None,
        }
    }

    pub fn error(id: impl Into<Option<RequestId>>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: Cow::Borrowed("2.0"),
            id: id.into(),
            result: None,
            error: Some(error),
        }
    }

    /// The response as `Ok(result)` or `Err(error)`. A response carrying
    /// neither (invalid JSON-RPC) comes back as an internal error.
    pub fn into_result(self) -> Result<Value, JsonRpcError> {
        match (self.result, self.error) {
            (_, Some(error)) => Err(error),
            (Some(result), None) => Ok(result),
            (None, None) => Err(JsonRpcError {
                code: error_codes::INTERNAL_ERROR,
                message: "response carried neither result nor error".into(),
                data: None,
            }),
        }
    }
}

/// JSON-RPC error codes, including the MCP-reserved allocations
/// (`2026-07-28` reserves `-32020..=-32099` for the specification).
pub mod error_codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    pub const HEADER_MISMATCH: i64 = -32020;
    pub const MISSING_REQUIRED_CLIENT_CAPABILITY: i64 = -32021;
    pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
}

/// The error member of an error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: error_codes::METHOD_NOT_FOUND,
            message: format!("unknown method: {method}"),
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: error_codes::INVALID_PARAMS,
            message: message.into(),
            data: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: error_codes::INTERNAL_ERROR,
            message: message.into(),
            data: None,
        }
    }
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

impl std::error::Error for JsonRpcError {}

/// Any message that can arrive on a transport, discriminated by shape:
/// `method` + `id` is a request, `method` alone is a notification, and
/// `result`/`error` is a response.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
    Response(JsonRpcResponse),
}

impl<'de> Deserialize<'de> for JsonRpcMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("expected a JSON-RPC object"))?;
        if object.contains_key("method") {
            if object.contains_key("id") {
                serde_json::from_value(value).map(Self::Request)
            } else {
                serde_json::from_value(value).map(Self::Notification)
            }
            .map_err(D::Error::custom)
        } else if object.contains_key("result") || object.contains_key("error") {
            serde_json::from_value(value)
                .map(Self::Response)
                .map_err(D::Error::custom)
        } else {
            Err(D::Error::custom(
                "object is neither a request, a notification, nor a response",
            ))
        }
    }
}

// --- identity and capabilities ---

/// Describes an MCP implementation (a client or a server).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    pub name: Cow<'static, str>,
    pub version: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icons: Option<Vec<Icon>>,
}

impl Implementation {
    pub fn new(name: impl Into<Cow<'static, str>>, version: impl Into<Cow<'static, str>>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            title: None,
            description: None,
            website_url: None,
            icons: None,
        }
    }
}

/// An optionally-sized icon for display in a user interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Icon {
    pub src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sizes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

/// Capabilities a client advertises. The leaf shapes this crate does not
/// interpret stay as raw JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elicitation: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Map<String, Value>>,
}

/// Capabilities a server advertises.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completions: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Map<String, Value>>,
}

impl ServerCapabilities {
    /// Capabilities advertising tools only — what this crate's serve loop offers.
    pub fn tools_only() -> Self {
        Self {
            tools: Some(ToolsCapability::default()),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
}

// --- lifecycle ---

/// `initialize` request params (protocol revisions through `2025-11-25`;
/// removed in `2026-07-28`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequestParams {
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: ClientCapabilities,
    pub client_info: Implementation,
}

/// `initialize` result. The server echoes the requested protocol version
/// when it supports it, and otherwise answers with the newest version it
/// does support; the client then decides whether to continue.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: ServerCapabilities,
    pub server_info: Implementation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Map<String, Value>>,
}

/// `server/discover` result (`2026-07-28`): the stateless replacement for
/// the handshake, and the backward-compatibility probe — a pre-`2026-07-28`
/// server answers it with method-not-found, telling the client to fall back
/// to `initialize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResult {
    pub supported_versions: Vec<String>,
    #[serde(default)]
    pub capabilities: ServerCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_scope: Option<CacheScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_type: Option<ResultType>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Map<String, Value>>,
}

/// Cache scope for a cacheable result (`2026-07-28`), analogous to HTTP
/// `Cache-Control: public` vs `private`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheScope {
    Public,
    Private,
}

/// Discriminates a result as final or interim.
///
/// `2026-07-28` requires this field on every result. The spec directs clients
/// to treat an absent value — from a server on an earlier revision — as
/// [`Complete`](ResultType::Complete). Unrecognized values from a future
/// revision are preserved in [`Other`](ResultType::Other) rather than failing
/// to parse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResultType {
    /// An ordinary, final result.
    #[default]
    Complete,
    /// An interim result: the server needs more input before it can finish.
    /// See [`InputRequiredResult`].
    InputRequired,
    #[serde(untagged)]
    Other(String),
}

/// Well-known `_meta` keys reserved by the specification.
///
/// Any prefix whose second label is `modelcontextprotocol` or `mcp` is
/// reserved for MCP use, so these must not be invented locally.
pub mod meta_keys {
    /// Required on every request (`2026-07-28`).
    pub const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
    /// Clients SHOULD send this on every request.
    pub const CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
    /// Required on every request. Declared per-request; servers MUST NOT infer
    /// it from prior requests.
    pub const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
    /// Servers SHOULD include this in every result.
    pub const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
    /// Per-request log level. Deprecated in `2026-07-28` along with Logging.
    pub const LOG_LEVEL: &str = "io.modelcontextprotocol/logLevel";
    /// Correlates a notification with the `subscriptions/listen` stream it
    /// arrived on.
    pub const SUBSCRIPTION_ID: &str = "io.modelcontextprotocol/subscriptionId";
}

// --- tools ---

/// Definition of a tool the client can call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    /// A JSON Schema object (`type: "object"` at the root; any JSON Schema
    /// 2020-12 keywords beyond that). Kept as raw JSON rather than a typed
    /// subset so nothing a server advertises is lost in a round-trip.
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icons: Option<Vec<Icon>>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Map<String, Value>>,
}

impl Tool {
    pub fn new(name: impl Into<String>, input_schema: Value) -> Self {
        Self {
            name: name.into(),
            input_schema,
            title: None,
            description: None,
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }
}

/// Behavior hints for a tool. All properties are hints — the spec warns
/// clients not to make tool-use decisions on them for untrusted servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

/// `tools/list` result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListToolsResult {
    pub tools: Vec<Tool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_scope: Option<CacheScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_type: Option<ResultType>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Map<String, Value>>,
}

/// `tools/call` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolRequestParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Map<String, Value>>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Map<String, Value>>,
}

/// `tools/call` result. A failure *of the tool* is reported here with
/// `is_error: true` so the model can see it and self-correct; a JSON-RPC
/// error response is reserved for failures of the protocol (unknown tool,
/// malformed params).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_type: Option<ResultType>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Map<String, Value>>,
}

impl CallToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::text(text)],
            result_type: Some(ResultType::Complete),
            ..Self::default()
        }
    }

    /// A result carrying both the model-facing content and the program-facing
    /// structured value produced by a [`ToolOutput`](crate::traits::ToolOutput).
    pub fn from_content(content: Vec<ContentBlock>, structured_content: Option<Value>) -> Self {
        Self {
            content,
            structured_content,
            result_type: Some(ResultType::Complete),
            ..Self::default()
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            is_error: Some(true),
            ..Self::text(text)
        }
    }
}

/// An interim `tools/call`, `prompts/get`, or `resources/read` result: the server needs more input
/// before it can finish, and the client is expected to supply it and re-issue the original request
/// as a *new* request.
///
/// This crate's serve loop never produces one but a client must be able to recognize one. At least
/// one of `input_requests` or `request_state` is always present; a `request_state`-only result is
/// the spec's load-shedding case and requires no declared client capability, so *any* client can
/// receive one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequiredResult {
    /// Server-initiated requests the client must fulfill, keyed by
    /// server-assigned identifiers. Values are `elicitation/create`,
    /// `sampling/createMessage`, or `roots/list` requests; the latter two are
    /// deprecated in `2026-07-28`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_requests: Option<Map<String, Value>>,
    /// Opaque server state to echo back verbatim on the retry. Clients MUST
    /// NOT inspect, parse, or modify it, and MUST NOT invent one when the
    /// server did not send one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_type: Option<ResultType>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Map<String, Value>>,
}

/// The two shapes a successful `tools/call` response can take.
///
/// Discriminated on `resultType`, treating absent as
/// [`Complete`](ResultType::Complete) per the spec's rule for servers on
/// earlier revisions. Without this discrimination a client would deserialize an
/// [`InputRequiredResult`] as a `CallToolResult` with empty `content` and
/// report a successful empty tool call.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ToolCallOutcome {
    Complete(CallToolResult),
    InputRequired(InputRequiredResult),
}

impl<'de> Deserialize<'de> for ToolCallOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let value = Value::deserialize(deserializer)?;
        let result_type = value.get("resultType").and_then(Value::as_str);
        if result_type == Some("input_required") {
            serde_json::from_value(value).map(Self::InputRequired)
        } else {
            serde_json::from_value(value).map(Self::Complete)
        }
        .map_err(D::Error::custom)
    }
}

// --- request context ---

/// What a request tells a tool about its caller.
///
/// `2026-07-28` removed the handshake, so protocol version and client
/// capabilities arrive in every request's `_meta` instead of once at
/// initialization. Capabilities are per-request by design: the spec forbids
/// servers from inferring them from prior requests, so this is rebuilt for
/// each call rather than cached.
///
/// Requests from earlier revisions carry none of this; every field is
/// therefore optional or defaulted.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// The protocol version the caller declared for this request.
    pub protocol_version: Option<String>,
    /// The caller's self-reported identity. Not verified by the protocol —
    /// for display, logging, and debugging only. Servers SHOULD NOT change
    /// behavior based on it, and MUST NOT use it for security decisions.
    pub client_info: Option<Implementation>,
    /// What the caller supports *for this request*. Empty means no optional
    /// capabilities.
    pub client_capabilities: ClientCapabilities,
}

impl RequestContext {
    /// Build a context from a request's `_meta` object.
    pub fn from_meta(meta: Option<&Map<String, Value>>) -> Self {
        let Some(meta) = meta else {
            return Self::default();
        };

        Self {
            protocol_version: meta
                .get(meta_keys::PROTOCOL_VERSION)
                .and_then(Value::as_str)
                .map(String::from),
            client_info: meta
                .get(meta_keys::CLIENT_INFO)
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok()),
            client_capabilities: meta
                .get(meta_keys::CLIENT_CAPABILITIES)
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default(),
        }
    }

    /// Pull the `_meta` out of a request's `params` and build a context.
    pub fn from_params(params: Option<&Value>) -> Self {
        Self::from_meta(
            params
                .and_then(|params| params.get("_meta"))
                .and_then(Value::as_object),
        )
    }

    /// Whether the caller declared support for elicitation on this request.
    /// A server MUST NOT send an elicitation `inputRequest` when this is false.
    pub fn supports_elicitation(&self) -> bool {
        self.client_capabilities.elicitation.is_some()
    }
}

// --- content ---

/// One block of tool-result or prompt content, discriminated by `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text", rename_all = "camelCase")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Annotations>,
        #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
        meta: Option<Map<String, Value>>,
    },
    #[serde(rename = "image", rename_all = "camelCase")]
    Image {
        /// Base64-encoded image data.
        data: String,
        mime_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Annotations>,
        #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
        meta: Option<Map<String, Value>>,
    },
    #[serde(rename = "audio", rename_all = "camelCase")]
    Audio {
        /// Base64-encoded audio data.
        data: String,
        mime_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Annotations>,
        #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
        meta: Option<Map<String, Value>>,
    },
    #[serde(rename = "resource_link", rename_all = "camelCase")]
    ResourceLink {
        uri: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        icons: Option<Vec<Icon>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Annotations>,
        #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
        meta: Option<Map<String, Value>>,
    },
    #[serde(rename = "resource", rename_all = "camelCase")]
    EmbeddedResource {
        resource: ResourceContents,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Annotations>,
        #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
        meta: Option<Map<String, Value>>,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            annotations: None,
            meta: None,
        }
    }
}

/// The contents of an embedded resource: textual or binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResourceContents {
    #[serde(rename_all = "camelCase")]
    Text {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Blob {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        /// Base64-encoded binary data.
        blob: String,
    },
}

/// Client-facing annotations on a content block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

// --- framework support ---

/// A described example attached to a tool's schema (`examples` keyword).
#[derive(Serialize, Deserialize, Debug)]
pub struct Example<T> {
    pub description: &'static str,
    #[serde(flatten)]
    pub item: T,
}
