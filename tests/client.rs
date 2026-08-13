//! The client surface on its own: what it puts on the wire, and how
//! negotiation resolves. The client driven against a real server is in
//! `tests/dispatch`.

#![cfg(feature = "client")]

use mcplease::{
    client::{ClientProtocol, Negotiation, NegotiationError, Step},
    types::{
        ClientCapabilities, Implementation, JsonRpcError, LATEST_PROTOCOL_VERSION, RequestContext,
        SUPPORTED_PROTOCOL_VERSIONS,
    },
};
use serde_json::json;

fn client() -> ClientProtocol {
    ClientProtocol::new(Implementation::new("test-client", "9.9.9"))
}

#[test]
fn every_request_carries_the_meta_a_server_rebuilds_its_context_from() {
    // The two halves of `_meta`: the client writes it here, and
    // `RequestContext::from_params` — what every server calls — reads it back.
    // Neither side is useful if they disagree about the reserved keys.
    let mut protocol = client().with_capabilities(ClientCapabilities {
        elicitation: Some(json!({})),
        ..ClientCapabilities::default()
    });

    let request = protocol.tools_call("search", &json!({ "query": "rust" }));
    let context = RequestContext::from_params(request.params.as_ref());

    assert_eq!(
        context.protocol_version.as_deref(),
        // Nothing has been negotiated yet, and a `2026-07-28` server must
        // still be told something on the very first request.
        Some(LATEST_PROTOCOL_VERSION)
    );
    let client_info = context.client_info.as_ref().expect("clientInfo");
    assert_eq!(client_info.name, "test-client");
    assert_eq!(client_info.version, "9.9.9");
    assert!(context.supports_elicitation());

    // …and the params the tool itself reads are undisturbed.
    let params = request.params.unwrap();
    assert_eq!(params["name"], "search");
    assert_eq!(params["arguments"]["query"], "rust");
}

#[test]
fn a_paramless_request_still_carries_meta() {
    let mut protocol = client();
    let request = protocol.request("ping", None);
    let context = RequestContext::from_params(request.params.as_ref());
    assert_eq!(context.client_info.unwrap().name, "test-client");
}

#[test]
fn ids_are_unique_per_client() {
    let mut protocol = client();
    let ids: Vec<_> = (0..3).map(|_| protocol.tools_list(None).id).collect();
    assert_eq!(ids[0], 1.into());
    assert_eq!(ids[1], 2.into());
    assert_eq!(ids[2], 3.into());
}

#[test]
fn notifications_carry_no_meta() {
    // There is no result to correlate and no per-request context to act on.
    let protocol = client();
    let notification = protocol.notification("notifications/initialized", None);
    assert!(notification.params.is_none());
}

/// Settle a negotiation whose `server/discover` answers with `offered`.
fn discovered(offered: &[&str]) -> Result<Step, NegotiationError> {
    let mut protocol = client();
    let (mut negotiation, _) = Negotiation::start(&mut protocol);
    negotiation.on_result(
        &mut protocol,
        Ok(json!({ "supportedVersions": offered, "capabilities": {} })),
    )
}

#[test]
fn negotiation_takes_the_newest_revision_both_sides_speak() {
    // Not the newest the server offers, and not the order it listed them in:
    // the newest this client also models.
    let Ok(Step::Done(negotiated)) = discovered(&["2024-11-05", "2077-01-01", "2025-06-18"]) else {
        panic!("expected a settled negotiation");
    };
    assert_eq!(negotiated.protocol_version, "2025-06-18");
    assert!(!negotiated.stateful);
}

#[test]
fn a_server_with_no_revision_in_common_fails_the_negotiation() {
    let Err(error) = discovered(&["2077-01-01"]) else {
        panic!("expected the negotiation to fail");
    };
    let message = error.to_string();
    assert!(message.contains("2077-01-01"), "{message}");
    assert!(
        message.contains(SUPPORTED_PROTOCOL_VERSIONS[0]),
        "{message}"
    );
}

#[test]
fn a_server_that_refuses_both_entry_points_reports_both() {
    let mut protocol = client();
    let (mut negotiation, _) = Negotiation::start(&mut protocol);
    let declined = Err(JsonRpcError::method_not_found("server/discover"));
    let Ok(Step::Send(_)) = negotiation.on_result(&mut protocol, declined) else {
        panic!("expected the fallback to initialize");
    };
    let refused = Err(JsonRpcError::internal("no thanks"));
    let Err(error) = negotiation.on_result(&mut protocol, refused) else {
        panic!("expected the negotiation to fail");
    };
    let message = error.to_string();
    assert!(message.contains("server/discover"), "{message}");
    assert!(message.contains("no thanks"), "{message}");
}

#[test]
fn an_unparseable_result_names_the_method_it_came_from() {
    let mut protocol = client();
    let (mut negotiation, _) = Negotiation::start(&mut protocol);
    let Err(error) = negotiation.on_result(&mut protocol, Ok(json!({ "nonsense": true }))) else {
        panic!("expected the negotiation to fail");
    };
    assert!(error.to_string().contains("server/discover"));
}
