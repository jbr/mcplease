use super::*;
use crate::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

fn request(method: &str, params: Option<Value>) -> JsonRpcMessage {
    JsonRpcMessage::Request(JsonRpcRequest::new(1i64, method, params))
}

/// A header lookup over a fixed set, in the shape a transport provides —
/// trillium's `get_str` and a `HashMap` both hand back `Option<&str>`.
fn headers<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<&'a str> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
    }
}

// --- what a client sends ---

#[test]
fn every_request_and_notification_carries_its_method() {
    assert_eq!(
        standard_headers(&request("tools/list", None)),
        [(MCP_METHOD, "tools/list".to_string())]
    );
    assert_eq!(
        standard_headers(&JsonRpcMessage::Notification(JsonRpcNotification::new(
            "notifications/initialized",
            None
        ))),
        [(MCP_METHOD, "notifications/initialized".to_string())]
    );
}

/// A response is not a request and has no `method` to mirror.
#[test]
fn a_response_carries_no_standard_headers() {
    let response = JsonRpcMessage::Response(JsonRpcResponse::success(1i64.into(), Value::Null));
    assert!(standard_headers(&response).is_empty());
}

#[test]
fn the_addressed_methods_carry_a_name() {
    assert_eq!(
        standard_headers(&request(
            "tools/call",
            Some(serde_json::json!({"name": "get_weather", "arguments": {}}))
        )),
        [
            (MCP_METHOD, "tools/call".to_string()),
            (MCP_NAME, "get_weather".to_string()),
        ]
    );
    assert_eq!(
        name_for(
            "resources/read",
            Some(&serde_json::json!({"uri": "file:///a/b.json"}))
        )
        .as_deref(),
        Some("file:///a/b.json")
    );
    assert_eq!(
        name_for("prompts/get", Some(&serde_json::json!({"name": "review"}))).as_deref(),
        Some("review")
    );
}

/// `tools/list` addresses no tool, and a malformed `params` yields nothing
/// rather than panicking — a server will reject the request on its own terms.
#[test]
fn everything_else_carries_no_name() {
    assert_eq!(name_for("tools/list", Some(&serde_json::json!({}))), None);
    assert_eq!(name_for("initialize", None), None);
    assert_eq!(name_for("tools/call", None), None);
    assert_eq!(
        name_for("tools/call", Some(&serde_json::json!({"name": 7}))),
        None
    );
}

// --- value encoding ---

/// The encoding examples from the specification's Value Encoding table.
#[test]
fn the_spec_encoding_examples() {
    for (original, encoded) in [
        ("us-west1", "us-west1"),
        ("Hello, 世界", "=?base64?SGVsbG8sIOS4lueVjA==?="),
        (" padded ", "=?base64?IHBhZGRlZCA=?="),
        ("line1\nline2", "=?base64?bGluZTEKbGluZTI=?="),
        ("=?base64?literal?=", "=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?="),
    ] {
        assert_eq!(encode_value(original), encoded, "encoding {original:?}");
        assert_eq!(decode_value(encoded), original, "decoding {encoded:?}");
    }
}

#[test]
fn safe_values_ride_raw() {
    for safe in [
        "tools/call",
        "get_weather",
        "file:///projects/myapp/config.json",
        "a-name_with.punctuation!",
        "",
    ] {
        assert_eq!(encode_value(safe), safe);
        assert_eq!(decode_value(safe), safe);
    }
}

/// Every byte length mod 3, so the padding arithmetic is covered rather than
/// assumed.
#[test]
fn encoding_round_trips_at_every_padding_length() {
    for original in [
        "é",
        "éé",
        "ééé",
        "éééé",
        "\u{1}",
        "\u{1}\u{2}",
        "\u{1}\u{2}\u{3}",
    ] {
        let encoded = encode_value(original);
        assert!(encoded.starts_with(SENTINEL_PREFIX), "{original:?}");
        assert_eq!(decode_value(&encoded), original, "round trip {original:?}");
    }
}

/// A value inside the markers that is not valid base64 is handed back as-is:
/// the caller is about to compare it, and garbage will simply not match.
#[test]
fn an_undecodable_payload_is_returned_unchanged() {
    assert_eq!(
        decode_value("=?base64?not*valid*?="),
        "=?base64?not*valid*?="
    );
    assert_eq!(decode_value("=?base64?"), "=?base64?");
}

// --- what a server checks ---

#[test]
fn matching_headers_validate() {
    let message = request(
        "tools/call",
        Some(serde_json::json!({"name": "get_weather", "arguments": {}})),
    );
    assert!(
        validate(
            &message,
            headers(&[("mcp-method", "tools/call"), ("mcp-name", "get_weather")])
        )
        .is_ok()
    );
}

/// Header names are case-insensitive in HTTP; a transport handing them over in
/// their original casing must still validate.
#[test]
fn header_lookup_is_case_insensitive() {
    let message = request("tools/list", None);
    assert!(validate(&message, headers(&[("Mcp-Method", "tools/list")])).is_ok());
}

/// A client declaring `2026-07-28` or later has no excuse: the standard
/// headers are part of the protocol it claims to speak.
#[test]
fn a_missing_method_header_is_rejected_on_a_modern_revision() {
    let error = validate(
        &request("tools/list", None),
        headers(&[(MCP_PROTOCOL_VERSION, "2026-07-28")]),
    )
    .unwrap_err();
    assert_eq!(error.code, error_codes::HEADER_MISMATCH);
    assert!(error.message.contains("mcp-method"), "{}", error.message);
    assert!(error.message.contains("missing"), "{}", error.message);
}

#[test]
fn a_missing_name_header_is_rejected_on_a_modern_revision() {
    let message = request(
        "tools/call",
        Some(serde_json::json!({"name": "get_weather"})),
    );
    let error = validate(
        &message,
        headers(&[
            (MCP_PROTOCOL_VERSION, "2026-07-28"),
            ("mcp-method", "tools/call"),
        ]),
    )
    .unwrap_err();
    assert_eq!(error.code, error_codes::HEADER_MISMATCH);
    assert!(error.message.contains("mcp-name"), "{}", error.message);
}

/// SEP-2243 postdates every revision through `2025-11-25`: a client declaring
/// one of those — or declaring nothing, which implies a legacy revision —
/// cannot be expected to send headers its protocol does not define.
#[test]
fn absent_headers_pass_on_a_pre_sep2243_revision() {
    let message = request(
        "tools/call",
        Some(serde_json::json!({"name": "get_weather"})),
    );
    assert!(validate(&message, headers(&[(MCP_PROTOCOL_VERSION, "2025-11-25")])).is_ok());
    assert!(validate(&message, headers(&[(MCP_PROTOCOL_VERSION, "2025-03-26")])).is_ok());
    assert!(validate(&message, headers(&[])).is_ok());
}

/// The version gates requiredness, not agreement: a mirror header that is
/// present and wrong is a mis-route waiting to happen in any era.
#[test]
fn a_present_header_must_agree_regardless_of_revision() {
    let error = validate(
        &request("tools/list", None),
        headers(&[
            (MCP_PROTOCOL_VERSION, "2025-11-25"),
            ("mcp-method", "tools/call"),
        ]),
    )
    .unwrap_err();
    assert_eq!(error.code, error_codes::HEADER_MISMATCH);
}

/// The security case the redundancy exists for: an intermediary routing on the
/// header while the server executes the body.
#[test]
fn a_disagreeing_header_is_rejected_with_both_values() {
    let message = request("tools/call", Some(serde_json::json!({"name": "bar"})));
    let error = validate(
        &message,
        headers(&[("mcp-method", "tools/call"), ("mcp-name", "foo")]),
    )
    .unwrap_err();
    assert_eq!(error.code, error_codes::HEADER_MISMATCH);
    assert!(error.message.contains("foo"), "{}", error.message);
    assert!(error.message.contains("bar"), "{}", error.message);
    let data = error.data.unwrap();
    assert_eq!(data["mismatch"]["header"], "foo");
    assert_eq!(data["mismatch"]["body"], "bar");
}

/// A client that had to encode a name must not be rejected for having done so:
/// the specification requires the server to decode before comparing.
#[test]
fn an_encoded_header_is_decoded_before_comparison() {
    let message = request(
        "resources/read",
        Some(serde_json::json!({"uri": "file:///Hello, 世界.json"})),
    );
    let encoded = encode_value("file:///Hello, 世界.json");
    assert!(encoded.starts_with(SENTINEL_PREFIX));
    assert!(
        validate(
            &message,
            headers(&[("mcp-method", "resources/read"), ("mcp-name", &encoded)])
        )
        .is_ok()
    );
}

/// The two halves are one derivation, so what a client sends always satisfies
/// what a server checks. This is the property the module exists for.
#[test]
fn what_a_client_sends_always_validates() {
    for message in [
        request("initialize", None),
        request("tools/list", None),
        request("server/discover", Some(serde_json::json!({}))),
        request(
            "tools/call",
            Some(serde_json::json!({"name": "get_weather", "arguments": {"a": 1}})),
        ),
        request("prompts/get", Some(serde_json::json!({"name": "réview"}))),
        request(
            "resources/read",
            Some(serde_json::json!({"uri": "file:///a b/ünïcödé.json"})),
        ),
        JsonRpcMessage::Notification(JsonRpcNotification::new("notifications/initialized", None)),
    ] {
        let sent = standard_headers(&message);
        let pairs: Vec<(&str, &str)> = sent.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert!(
            validate(&message, headers(&pairs)).is_ok(),
            "client-sent headers failed server validation for {sent:?}"
        );
    }
}

/// A response has no method to disagree about, so it validates trivially — a
/// server must not reject one for lacking headers it could never carry.
#[test]
fn a_response_validates_without_headers() {
    let response = JsonRpcMessage::Response(JsonRpcResponse::success(1i64.into(), Value::Null));
    assert!(validate(&response, headers(&[])).is_ok());
}
