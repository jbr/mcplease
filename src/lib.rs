// Label every feature-gated item in the rendered docs with the feature that
// enables it. docs.rs builds with `--cfg docsrs` (see `[package.metadata.docs.rs]`);
// an ordinary build sees none of this.
#![cfg_attr(docsrs, feature(doc_cfg))]

//! A simple framework for writing Model Context Protocol servers and clients.
//!
//! The [`types`] and [`headers`] modules are unconditional — both sides of the
//! protocol need all of them, and they cost nothing but serde. `headers` is
//! shared rather than split because a client deriving what to send and a server
//! checking what arrived are the *same* derivation; splitting them is how they
//! drift. Everything else sits behind a feature
//! naming either a *side* of the protocol or a *transport*; see the `[features]`
//! table in `Cargo.toml`. The default is everything.
//!
//! A server that speaks a transport this crate does not implement wants
//! `default-features = false, features = ["server"]`: that is [`traits`], the
//! [`tools!`] macro, and [`handle_request`], which answers a decoded request
//! without doing any I/O.
//!
//! Dispatch is async and concurrent, but this crate names no runtime and
//! starts none: the binary brings one. [`calls`] runs the stdio transport over
//! any `futures-lite` `AsyncRead`/`AsyncWrite` pair and hands each admitted
//! `tools/call` back as a future for the binary to spawn. A transport supplies a
//! [`Notifier`](types::Notifier) and a cancellation flag when it builds each
//! request's [`RequestContext`](types::RequestContext); everything a tool can
//! say mid-call goes through there.
//!
//! A *client* wants `default-features = false, features = ["client"]`: the
//! [`client`] module, which is the same split from the other side — it builds
//! requests and classifies the messages that arrive, and leaves framing,
//! connections, and authorization to the transport. It pulls no dependencies
//! this crate does not already have.
//!
//! The dependencies re-exported at the root are there for the macros to name
//! and for a downstream tool to use without duplicating the version
//! requirement. Each is present only under the features that pull it.

#[cfg(feature = "server")]
#[macro_use]
mod macros;

#[cfg(feature = "server")]
pub mod claude_channel;
#[cfg(feature = "cli")]
mod cli;
#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "stdio")]
mod stdio;

pub mod headers;
#[cfg(feature = "server")]
pub mod traits;
pub mod types;

#[cfg(feature = "server")]
pub use anyhow;
#[cfg(feature = "cli")]
pub use clap;
#[cfg(feature = "cli")]
pub use cli::Cli;
#[cfg(feature = "server")]
pub use fieldwork;
pub use log;
#[cfg(feature = "server")]
pub use schemars;
pub use serde;
pub use serde_json;
#[cfg(feature = "server")]
pub use server::{
    DEFAULT_MAX_CONCURRENT_CALLS, DEFAULT_TOOLS_TTL_MS, ServerConfig, answer_stateless,
    answer_tool_call, handle_request,
};
#[cfg(feature = "stdio")]
pub use stdio::{Call, Calls, ConnectionNotifier, calls};
pub use swansong;
