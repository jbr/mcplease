//! Claude Code's *channels*: events a server pushes into a running Claude Code
//! session, outside any request. A Claude Code extension, not part of MCP; the
//! contract is Anthropic's channels reference, copied under
//! `spec/claude-code/` with the date it was fetched.
//!
//! A channel server does two things:
//!
//! 1. Declares [`CAPABILITY`], with an empty object as its value, under `capabilities.experimental`
//!    — [`ServerConfig::with_experimental_capability`].
//! 2. Emits a [`ChannelEvent`] as a `notifications/claude/channel` notification whenever something
//!    happens. On stdio that goes through [`Calls::notifier`] or a tool's
//!    [`RequestContext::connection_notifier`].
//!
//! The model reads each event as `<channel source="<server name>" k="v">content</channel>`.
//! Several events arriving while Claude is busy are delivered together on its
//! next turn; one arriving while it is idle starts a turn.
//!
//! # What the server cannot see
//!
//! Delivery is unacknowledged. Claude Code drops the event silently, with no
//! error to the server, when the session did not opt the server in as a
//! channel (`--channels`, or `--dangerously-load-development-channels
//! server:<name>` for one off the allowlist), or when organization policy
//! blocks channels. A server that needs to know an event landed exposes a tool
//! the model calls to say so.
//!
//! # Revisions
//!
//! A channel event is a notification tied to no request. Revisions through
//! `2025-11-25` allow a server to send those once the `initialize` handshake
//! is done; `2026-07-28` does not — over stdio a server's notifications there
//! either belong to an in-flight request or to a `subscriptions/listen`
//! stream. Claude Code accordingly does not register a channel server that
//! negotiates `2026-07-28`, and the stdio [`Calls::notifier`] stays silent on
//! a connection that never completed a handshake.
//!
//! [`ServerConfig::with_experimental_capability`]: crate::ServerConfig::with_experimental_capability
//! [`Calls::notifier`]: crate::Calls::notifier
//! [`RequestContext::connection_notifier`]: crate::types::RequestContext::connection_notifier

use crate::types::JsonRpcNotification;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The `capabilities.experimental` key that declares a channel server. Its
/// value is an empty object.
pub const CAPABILITY: &str = "claude/channel";

/// The notification method an event is sent as.
pub const METHOD: &str = "notifications/claude/channel";

/// One event for the session: a body, and attributes for the `<channel>` tag
/// it is delivered in.
///
/// Attributes are kept in key order, so the same event always serializes to
/// the same bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelEvent {
    content: String,
    meta: BTreeMap<String, String>,
}

impl ChannelEvent {
    /// An event with this body and no attributes.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            meta: BTreeMap::new(),
        }
    }

    /// Add an attribute to the `<channel>` tag, for routing context the model
    /// reads beside the body: a sender, an id, a severity.
    ///
    /// Claude Code keeps only keys made of ASCII letters, digits and
    /// underscores, and drops any other key silently. A key it would drop is
    /// dropped here instead, with a warning in this server's log, so the loss
    /// is visible on the side that can fix it.
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        if is_identifier(&key) {
            self.meta.insert(key, value.into());
        } else {
            log::warn!("dropping channel attribute {key:?}: keys must be letters, digits and _");
        }
        self
    }

    /// The event body.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// The tag attributes, in key order.
    pub fn meta(&self) -> &BTreeMap<String, String> {
        &self.meta
    }

    /// The notification that carries this event.
    pub fn into_notification(self) -> JsonRpcNotification {
        let mut params = Map::new();
        params.insert("content".into(), Value::String(self.content));
        if !self.meta.is_empty() {
            params.insert("meta".into(), json!(self.meta));
        }
        JsonRpcNotification::new(METHOD, Some(Value::Object(params)))
    }
}

impl From<ChannelEvent> for JsonRpcNotification {
    fn from(event: ChannelEvent) -> Self {
        event.into_notification()
    }
}

/// Whether Claude Code keeps `key` as a tag attribute.
fn is_identifier(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
