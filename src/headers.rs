//! The standard request headers HTTP transports carry from `2026-07-28`
//! onward (SEP-2243), and the validation a server owes them.
//!
//! `Mcp-Method` mirrors the body's `method` on every request; `Mcp-Name`
//! mirrors `params.name` or `params.uri` on the three methods that address a
//! specific tool, prompt, or resource. Both are **required for compliance**.
//!
//! Their whole purpose is redundancy: an intermediary — a load balancer, a
//! router, a rate limiter — can act on the header without parsing the body, so
//! the two must agree or different components end up working from different
//! sources of truth. That is why the client's "what should I send" and the
//! server's "does this match" are the *same derivation*, and why they live in
//! one module rather than one on each side of the crate.
//!
//! ## This is not part of `handle_request`
//!
//! [`crate::handle_request`] takes a decoded request and never sees a header —
//! deliberately, since it is transport-agnostic and does no I/O. Headers belong
//! to the transport, so the transport is what calls [`validate`], which is a
//! free function for exactly that reason.
//!
//! ## Both halves of a wire, one derivation
//!
//! ```no_run
//! # use mcplease::{headers, types::JsonRpcMessage};
//! # fn client(message: &JsonRpcMessage) {
//! // Client: send what the body implies.
//! for (name, value) in headers::standard_headers(message) {
//!     // request.set_header(name, value)
//!     let _ = (name, value);
//! }
//! # }
//! # fn server(message: &JsonRpcMessage, request_headers: &std::collections::HashMap<String, String>) {
//! // Server: check the headers that arrived against the same derivation.
//! if let Err(error) = headers::validate(message, |name| {
//!     request_headers.get(name).map(String::as_str)
//! }) {
//!     // 400 Bad Request + JsonRpcResponse::error(id, error)
//!     let _ = error;
//! }
//! # }
//! ```

use crate::types::{JsonRpcError, JsonRpcMessage, error_codes};
use serde_json::Value;
use std::borrow::Cow;

/// The header naming the body's `method`. Required on every request.
pub const MCP_METHOD: &str = "mcp-method";
/// The header naming the addressed tool, prompt, or resource.
pub const MCP_NAME: &str = "mcp-name";

/// The marker wrapping a base64-encoded header value. Case-sensitive, and
/// lowercase exactly as shown.
const SENTINEL_PREFIX: &str = "=?base64?";
const SENTINEL_SUFFIX: &str = "?=";

/// The headers a client must send alongside `message`, already encoded.
///
/// A response carries no `method` and yields nothing — these headers describe
/// a request. Header names are lowercase, which HTTP treats as equivalent to
/// any other casing and which most header maps want anyway.
pub fn standard_headers(message: &JsonRpcMessage) -> Vec<(&'static str, String)> {
    let Some((method, params)) = method_and_params(message) else {
        return Vec::new();
    };
    let mut headers = vec![(MCP_METHOD, encode_value(method))];
    if let Some(name) = name_for(method, params) {
        headers.push((MCP_NAME, encode_value(&name)));
    }
    headers
}

/// The `Mcp-Name` source value for a method, or `None` if that method does not
/// address a specific thing.
///
/// `params.name` for a tool or prompt, `params.uri` for a resource. Nothing
/// else carries one — `tools/list` addresses no tool.
pub fn name_for(method: &str, params: Option<&Value>) -> Option<String> {
    let field = match method {
        "tools/call" | "prompts/get" => "name",
        "resources/read" => "uri",
        _ => return None,
    };
    Some(params?.get(field)?.as_str()?.to_string())
}

/// Check that the headers a request arrived with agree with its body.
///
/// `header` looks a header up by lowercase name. The error is ready to return
/// as the JSON-RPC body of a `400 Bad Request`, which is what the specification
/// requires for a validation failure.
///
/// Only requests and notifications are checked; a response carries no `method`
/// to disagree about. A message whose required header is *absent* fails the
/// same way one that mismatches does — the specification lists both as
/// validation failures, and a missing header is exactly the case an
/// intermediary would have had nothing to route on.
pub fn validate<'a>(
    message: &JsonRpcMessage,
    header: impl Fn(&str) -> Option<&'a str>,
) -> Result<(), JsonRpcError> {
    let Some((method, params)) = method_and_params(message) else {
        return Ok(());
    };
    check(MCP_METHOD, method, header(MCP_METHOD))?;
    if let Some(name) = name_for(method, params) {
        check(MCP_NAME, &name, header(MCP_NAME))?;
    }
    Ok(())
}

/// One header against one body value, decoding the sentinel encoding first —
/// the specification requires servers to decode before comparing, so a client
/// that had to encode a name is not rejected for having done so.
fn check(name: &'static str, expected: &str, actual: Option<&str>) -> Result<(), JsonRpcError> {
    let Some(actual) = actual else {
        return Err(mismatch(
            format!("Header mismatch: required header {name} is missing"),
            name,
            None,
            expected,
        ));
    };
    let decoded = decode_value(actual);
    if decoded == expected {
        return Ok(());
    }
    Err(mismatch(
        format!(
            "Header mismatch: {name} header value {decoded:?} does not match body value \
             {expected:?}"
        ),
        name,
        Some(&decoded),
        expected,
    ))
}

fn mismatch(message: String, name: &'static str, header: Option<&str>, body: &str) -> JsonRpcError {
    JsonRpcError {
        code: error_codes::HEADER_MISMATCH,
        message,
        // Beyond what the specification requires, and cheap: a client fixing
        // this wants the two values side by side more than it wants to parse
        // them back out of the message.
        data: Some(serde_json::json!({
            "mismatch": { "name": name, "header": header, "body": body }
        })),
    }
}

/// A header value per the specification's Value Encoding rules: as-is when it
/// can ride raw, and `=?base64?<base64 of the UTF-8 bytes>?=` when it cannot.
///
/// A plain-ASCII value that merely *looks* encoded is encoded too, so a server
/// cannot mistake a literal `=?base64?…?=` for a marker.
pub fn encode_value(value: &str) -> String {
    if is_header_safe(value) {
        return value.to_string();
    }
    format!(
        "{SENTINEL_PREFIX}{}{SENTINEL_SUFFIX}",
        base64_encode(value.as_bytes())
    )
}

/// The inverse of [`encode_value`]: unwrap a sentinel-encoded value, or hand
/// back what was there.
///
/// A malformed payload inside the markers is returned unchanged rather than
/// erroring — the caller is about to compare it to an expected value, and a
/// value that does not decode simply will not match.
pub fn decode_value(value: &str) -> Cow<'_, str> {
    let Some(inner) = value
        .strip_prefix(SENTINEL_PREFIX)
        .and_then(|rest| rest.strip_suffix(SENTINEL_SUFFIX))
    else {
        return Cow::Borrowed(value);
    };
    match base64_decode(inner).and_then(|bytes| String::from_utf8(bytes).ok()) {
        Some(decoded) => Cow::Owned(decoded),
        None => Cow::Borrowed(value),
    }
}

/// Whether a value can ride in a header raw: visible ASCII, space, or tab
/// (RFC 9110), with no leading or trailing whitespace that could be stripped in
/// transit, and not something a server would read as an encoding marker.
fn is_header_safe(value: &str) -> bool {
    let looks_encoded = value.starts_with(SENTINEL_PREFIX) && value.ends_with(SENTINEL_SUFFIX);
    !looks_encoded
        && value
            .bytes()
            .all(|b| (0x20..=0x7e).contains(&b) || b == 0x09)
        && !value.starts_with([' ', '\t'])
        && !value.ends_with([' ', '\t'])
}

fn method_and_params(message: &JsonRpcMessage) -> Option<(&str, Option<&Value>)> {
    match message {
        JsonRpcMessage::Request(request) => Some((&request.method, request.params.as_ref())),
        JsonRpcMessage::Notification(notification) => {
            Some((&notification.method, notification.params.as_ref()))
        }
        JsonRpcMessage::Response(_) => None,
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (RFC 4648 §4), which is what the sentinel
/// format carries.
///
/// Hand-rolled rather than taken as a dependency: this crate's unconditional
/// dependencies are serde and a log facade, and the whole of base64 for two
/// header values would be the first thing to break that. It is thirty lines and
/// pinned against the specification's own encoding examples.
fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - i * 6)) as usize & 0x3f] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        let value = ALPHABET.iter().position(|&c| c == byte)? as u32;
        buffer = buffer << 6 | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod test;
