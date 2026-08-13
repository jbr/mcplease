//! Transport-agnostic client protocol — the mirror of [`handle_request`].
//!
//! [`handle_request`] answers a decoded request without doing any I/O, which
//! leaves a server transport responsible for framing alone. This module is the
//! same split from the other side: [`ClientProtocol`] *builds* requests and
//! decides what to do with each message that arrives, without reading or
//! writing anything. A transport is again only framing:
//!
//! ```ignore
//! let request = protocol.request("tools/list", None);
//! transport.write(&JsonRpcMessage::Request(request.clone()))?;
//! loop {
//!     match protocol.on_message(transport.read()?) {
//!         Reaction::Result { id, result } if id == request.id => break result,
//!         Reaction::Reply(response) => transport.write(&response.into())?,
//!         _ => continue,
//!     }
//! }
//! ```
//!
//! That loop is identical whether the frames are newline-delimited JSON over a
//! subprocess's pipes or events on a streamable-HTTP SSE stream, which is why
//! there is no transport trait here and no sync/async split: the part that
//! differs between transports is the part a transport was always going to
//! write, and the part that does not differ needs neither I/O nor a runtime.
//!
//! Nothing here knows about connections, headers, authorization, retries, or
//! process lifetimes. Those are the transport's, and stay the transport's.
//!
//! [`handle_request`]: crate::handle_request

use crate::types::{
    CacheScope, ClientCapabilities, DiscoverResult, Implementation, InitializeRequestParams,
    InitializeResult, JsonRpcError, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, LATEST_HANDSHAKE_PROTOCOL_VERSION, LATEST_PROTOCOL_VERSION, RequestId,
    SUPPORTED_PROTOCOL_VERSIONS, ServerCapabilities, meta_keys,
};
use serde_json::{Map, Value, json};

/// A client's half of the protocol: request construction, `_meta` stamping,
/// and the disposition of every message that arrives.
///
/// Holds no connection, so one of these can outlive a reconnect, and a client
/// with several connections can hold one per connection or share a single
/// counter — the ids it allocates are unique per instance.
#[derive(Debug, Clone)]
pub struct ClientProtocol {
    client_info: Implementation,
    capabilities: ClientCapabilities,
    protocol_version: Option<String>,
    next_id: i64,
}

impl ClientProtocol {
    /// A client that declares no optional capabilities. A server MUST NOT ask
    /// such a client for elicitation or sampling.
    pub fn new(client_info: Implementation) -> Self {
        Self {
            client_info,
            capabilities: ClientCapabilities::default(),
            protocol_version: None,
            next_id: 0,
        }
    }

    /// Declare what this client supports. Capabilities are per-request in
    /// `2026-07-28` — the spec forbids a server from inferring them from
    /// earlier requests — so they are stamped onto every request, not
    /// announced once.
    pub fn with_capabilities(mut self, capabilities: ClientCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// The negotiated revision, once [`Negotiation`] has settled it.
    pub fn protocol_version(&self) -> Option<&str> {
        self.protocol_version.as_deref()
    }

    /// The context a server on any revision will reconstruct from this
    /// client's requests — the client-side counterpart of
    /// [`RequestContext::from_params`](crate::types::RequestContext::from_params).
    pub fn request_context(&self) -> crate::types::RequestContext {
        crate::types::RequestContext {
            protocol_version: Some(self.declared_version().to_string()),
            client_info: Some(self.client_info.clone()),
            client_capabilities: self.capabilities.clone(),
        }
    }

    /// The version to declare on a request: the negotiated one, or — before
    /// negotiation settles, when a `2026-07-28` server must already be told
    /// something — the newest revision these types model.
    fn declared_version(&self) -> &str {
        self.protocol_version
            .as_deref()
            .unwrap_or(LATEST_PROTOCOL_VERSION)
    }

    /// The `_meta` every request carries: protocol version, client identity,
    /// and capabilities.
    ///
    /// `2026-07-28` requires this on every request; earlier revisions carry
    /// none of it and ignore unknown `_meta` keys, so it is stamped
    /// unconditionally rather than switched on the negotiated revision. The
    /// one exception is `initialize`, whose params carry the same information
    /// as ordinary fields — see [`Negotiation`].
    fn meta(&self) -> Map<String, Value> {
        json!({
            meta_keys::PROTOCOL_VERSION: self.declared_version(),
            meta_keys::CLIENT_INFO: self.client_info,
            meta_keys::CLIENT_CAPABILITIES: self.capabilities,
        })
        .as_object()
        .cloned()
        .unwrap_or_default()
    }

    /// Allocate an id and build a request, stamping the client `_meta` into
    /// its params. Params that are not an object are left alone — there is
    /// nowhere to put `_meta` — which the spec's own request shapes never do.
    pub fn request(&mut self, method: impl Into<String>, params: Option<Value>) -> JsonRpcRequest {
        self.next_id += 1;
        let params = match params {
            Some(Value::Object(mut params)) => {
                params.insert("_meta".into(), Value::Object(self.meta()));
                Some(Value::Object(params))
            }
            None => Some(json!({ "_meta": self.meta() })),
            other => other,
        };
        JsonRpcRequest::new(RequestId::Integer(self.next_id), method, params)
    }

    /// A one-way message. Notifications carry no `_meta`: there is no result
    /// to correlate and no per-request context for a server to act on.
    pub fn notification(
        &self,
        method: impl Into<String>,
        params: Option<Value>,
    ) -> JsonRpcNotification {
        JsonRpcNotification::new(method, params)
    }

    /// `tools/list`, optionally continuing a paginated listing.
    pub fn tools_list(&mut self, cursor: Option<&str>) -> JsonRpcRequest {
        let params = cursor.map(|cursor| json!({ "cursor": cursor }));
        self.request("tools/list", params)
    }

    /// `tools/call`, where `arguments` is the tool's input object.
    ///
    /// This builds the first attempt only. `2026-07-28`'s multi round-trip
    /// retry — re-issuing with `inputResponses` and the server's opaque
    /// `requestState` after an
    /// [`InputRequiredResult`](crate::types::InputRequiredResult) — is not
    /// built here: this crate's serve loop never issues one, so the retry
    /// shape would ship untested against any real server. A client that meets
    /// one can recognize it (see
    /// [`ToolCallOutcome`](crate::types::ToolCallOutcome)) and build the retry
    /// with [`request`](Self::request).
    pub fn tools_call(&mut self, name: &str, arguments: &Value) -> JsonRpcRequest {
        self.request(
            "tools/call",
            Some(json!({ "name": name, "arguments": arguments })),
        )
    }

    /// What a transport should do with a message it just read.
    ///
    /// Takes `&mut self` because a future revision may have the client learn
    /// from server traffic; today it mutates nothing.
    pub fn on_message(&mut self, message: JsonRpcMessage) -> Reaction {
        match message {
            JsonRpcMessage::Response(response) => match response.id.clone() {
                Some(id) => Reaction::Result {
                    id,
                    result: response.into_result(),
                },
                // A response the server could not attribute to a request.
                // There is nothing to correlate it with.
                None => {
                    log::debug!("dropping a response with no id");
                    Reaction::Ignore
                }
            },
            JsonRpcMessage::Request(request) => {
                let response = if request.method == "ping" {
                    JsonRpcResponse::success(request.id, json!({}))
                } else {
                    // Every server-initiated method belongs to a capability
                    // this client did not declare, so the honest answer is
                    // that the method is not implemented here.
                    log::debug!("declining server-initiated {}", request.method);
                    JsonRpcResponse::error(
                        request.id,
                        JsonRpcError::method_not_found(&request.method),
                    )
                };
                Reaction::Reply(response)
            }
            JsonRpcMessage::Notification(notification) => {
                log::debug!("ignoring server notification {}", notification.method);
                Reaction::Ignore
            }
        }
    }
}

/// What a transport should do with a message [`ClientProtocol::on_message`]
/// just classified.
#[derive(Debug, Clone)]
pub enum Reaction {
    /// A response arrived. The transport compares `id` against the requests it
    /// has outstanding: its own, or one it abandoned and should drop.
    Result {
        id: RequestId,
        result: Result<Value, JsonRpcError>,
    },
    /// A reply the transport should send if it has a channel to send it on.
    /// Advisory: a transport reading a per-request SSE stream has no way to
    /// answer on that stream and may drop it.
    Reply(JsonRpcResponse),
    /// Nothing to do.
    Ignore,
}

/// What negotiation learned about a server.
#[derive(Debug, Clone)]
pub struct Negotiated {
    /// The revision both sides will speak.
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    /// Present when the server identified itself; `server/discover` does not
    /// carry an identity, so a stateless negotiation leaves this `None` until
    /// a result's `_meta` supplies one.
    pub server_info: Option<Implementation>,
    /// Guidance the server offers about how to use it.
    pub instructions: Option<String>,
    /// How long a `tools/list` result may be considered fresh, and whether a
    /// shared cache may serve it across authorization contexts. Only
    /// `server/discover` reports these.
    pub tools_ttl_ms: Option<u64>,
    pub tools_cache_scope: Option<CacheScope>,
    /// True when the server was reached through the `initialize` handshake
    /// rather than `server/discover`, and is therefore keeping session state.
    pub stateful: bool,
}

impl Negotiated {
    /// The confirmation a stateful server is waiting for. Send it before the
    /// first ordinary request; a stateless server needs nothing, and this is
    /// `None`.
    pub fn initialized_notification(&self) -> Option<JsonRpcNotification> {
        self.stateful
            .then(|| JsonRpcNotification::new("notifications/initialized", None))
    }
}

/// The `server/discover` → `initialize` fallback, as a state machine that
/// performs no I/O.
///
/// `2026-07-28` replaced the handshake with a stateless `server/discover`
/// probe, and made that probe the backward-compatibility test: a server on an
/// earlier revision answers it with method-not-found, which is how a client
/// learns to fall back. Deployed servers overwhelmingly still want the
/// handshake, so every client needs both paths and the rule for choosing.
///
/// The driver is the same shape as the message loop:
///
/// ```ignore
/// let (mut negotiation, mut request) = Negotiation::start(&mut protocol);
/// let negotiated = loop {
///     let result = transport.round_trip(request).await;
///     match negotiation.on_result(&mut protocol, result)? {
///         Step::Send(next) => request = next,
///         Step::Done(negotiated) => break *negotiated,
///     }
/// };
/// if let Some(confirm) = negotiated.initialized_notification() {
///     transport.notify(confirm).await?;
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Negotiation {
    /// The error `server/discover` came back with, kept so that a failing
    /// `initialize` can report both halves rather than only the second.
    discover_error: Option<JsonRpcError>,
    stage: Stage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Discovering,
    Initializing,
}

/// One move in a [`Negotiation`].
#[derive(Debug, Clone)]
pub enum Step {
    /// Send this request and feed its result back to
    /// [`Negotiation::on_result`].
    Send(JsonRpcRequest),
    /// Negotiation settled.
    Done(Box<Negotiated>),
}

impl Negotiation {
    /// Begin with the `server/discover` probe.
    pub fn start(protocol: &mut ClientProtocol) -> (Self, JsonRpcRequest) {
        let request = protocol.request("server/discover", None);
        (
            Self {
                discover_error: None,
                stage: Stage::Discovering,
            },
            request,
        )
    }

    /// Take the result of the request this negotiation last handed out.
    ///
    /// A `server/discover` that comes back as *any* JSON-RPC error falls back
    /// to `initialize`. The spec describes method-not-found as the signal, but
    /// a server that rejects an unknown method some other way is equally a
    /// server that does not speak `2026-07-28`, and the discarded error is
    /// carried into the failure message if `initialize` fails too.
    pub fn on_result(
        &mut self,
        protocol: &mut ClientProtocol,
        result: Result<Value, JsonRpcError>,
    ) -> Result<Step, NegotiationError> {
        match (self.stage, result) {
            (Stage::Discovering, Ok(value)) => {
                let discovered: DiscoverResult = serde_json::from_value(value)
                    .map_err(|e| NegotiationError::Malformed("server/discover", e.to_string()))?;
                let versions: Vec<&str> = discovered
                    .supported_versions
                    .iter()
                    .map(String::as_str)
                    .collect();
                // Newest revision both sides support. The server's own
                // ordering is not authoritative.
                let protocol_version = SUPPORTED_PROTOCOL_VERSIONS
                    .iter()
                    .find(|supported| versions.contains(supported))
                    .ok_or_else(|| {
                        NegotiationError::NoSharedVersion(discovered.supported_versions.clone())
                    })?
                    .to_string();
                protocol.protocol_version = Some(protocol_version.clone());
                Ok(Step::Done(Box::new(Negotiated {
                    protocol_version,
                    capabilities: discovered.capabilities,
                    server_info: server_info_from_meta(discovered.meta.as_ref()),
                    instructions: discovered.instructions,
                    tools_ttl_ms: discovered.ttl_ms,
                    tools_cache_scope: discovered.cache_scope,
                    stateful: false,
                })))
            }
            (Stage::Discovering, Err(error)) => {
                log::debug!("server/discover was declined ({error}); falling back to initialize");
                self.discover_error = Some(error);
                self.stage = Stage::Initializing;
                let params = InitializeRequestParams {
                    protocol_version: LATEST_HANDSHAKE_PROTOCOL_VERSION.into(),
                    capabilities: protocol.capabilities.clone(),
                    client_info: protocol.client_info.clone(),
                };
                let params = serde_json::to_value(params)
                    .map_err(|e| NegotiationError::Malformed("initialize", e.to_string()))?;
                // `initialize` carries version, identity, and capabilities as
                // ordinary params, so it is the one request built without the
                // `_meta` stamp — the two would be redundant, and a server on
                // an older revision reads only the params.
                protocol.next_id += 1;
                Ok(Step::Send(JsonRpcRequest::new(
                    RequestId::Integer(protocol.next_id),
                    "initialize",
                    Some(params),
                )))
            }
            (Stage::Initializing, Ok(value)) => {
                let initialized: InitializeResult = serde_json::from_value(value)
                    .map_err(|e| NegotiationError::Malformed("initialize", e.to_string()))?;
                if !SUPPORTED_PROTOCOL_VERSIONS.contains(&initialized.protocol_version.as_str()) {
                    return Err(NegotiationError::NoSharedVersion(vec![
                        initialized.protocol_version,
                    ]));
                }
                protocol.protocol_version = Some(initialized.protocol_version.clone());
                Ok(Step::Done(Box::new(Negotiated {
                    protocol_version: initialized.protocol_version,
                    capabilities: initialized.capabilities,
                    server_info: Some(initialized.server_info),
                    instructions: initialized.instructions,
                    tools_ttl_ms: None,
                    tools_cache_scope: None,
                    stateful: true,
                })))
            }
            (Stage::Initializing, Err(error)) => {
                Err(NegotiationError::Declined(Box::new(Declined {
                    discover: self.discover_error.clone(),
                    initialize: error,
                })))
            }
        }
    }
}

/// The identity a server stamps into every result's `_meta`.
fn server_info_from_meta(meta: Option<&Map<String, Value>>) -> Option<Implementation> {
    meta?
        .get(meta_keys::SERVER_INFO)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

/// Why a [`Negotiation`] could not settle.
#[derive(Debug, Clone)]
pub enum NegotiationError {
    /// A result did not parse as the shape its method promises.
    Malformed(&'static str, String),
    /// No revision this client models appears in what the server offers.
    NoSharedVersion(Vec<String>),
    /// The server refused both entry points. Boxed to keep the error — which
    /// rides in every negotiation `Result` — small.
    Declined(Box<Declined>),
}

/// What each entry point answered when neither worked.
#[derive(Debug, Clone)]
pub struct Declined {
    /// Absent only if `server/discover` was never reached.
    pub discover: Option<JsonRpcError>,
    pub initialize: JsonRpcError,
}

impl std::fmt::Display for NegotiationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(method, error) => write!(f, "unparseable {method} result: {error}"),
            Self::NoSharedVersion(offered) => write!(
                f,
                "no shared protocol version: server offers [{}], this client speaks [{}]",
                offered.join(", "),
                SUPPORTED_PROTOCOL_VERSIONS.join(", ")
            ),
            Self::Declined(declined) => match &declined.discover {
                Some(discover) => write!(
                    f,
                    "server declined both entry points: server/discover: {discover}; initialize: \
                     {}",
                    declined.initialize
                ),
                None => write!(f, "server declined initialize: {}", declined.initialize),
            },
        }
    }
}

impl std::error::Error for NegotiationError {}
