// Label every feature-gated item in the rendered docs with the feature that
// enables it. docs.rs builds with `--cfg docsrs` (see `[package.metadata.docs.rs]`);
// an ordinary build sees none of this.
#![cfg_attr(docsrs, feature(doc_cfg))]

//! A simple framework for writing Model Context Protocol servers.
//!
//! The [`types`] module is unconditional — both sides of the protocol need all
//! of it, and it costs nothing but serde. Everything else sits behind a feature
//! naming either a *side* of the protocol or a *transport*; see the `[features]`
//! table in `Cargo.toml`. The default is everything.
//!
//! A server that speaks a transport this crate does not implement wants
//! `default-features = false, features = ["server"]`: that is [`traits`], the
//! [`tools!`] macro, and [`handle_request`], which answers a decoded request
//! without doing any I/O.
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

#[cfg(feature = "cli")]
mod cli;
#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "stdio")]
mod stdio;

#[cfg(feature = "session")]
pub mod session;
#[cfg(feature = "server")]
pub mod traits;
pub mod types;

#[cfg(any(feature = "server", feature = "session"))]
pub use anyhow;
#[cfg(feature = "cli")]
pub use clap;
#[cfg(feature = "cli")]
pub use cli::run;
#[cfg(any(feature = "cli", feature = "session"))]
pub use dirs;
#[cfg(feature = "server")]
pub use fieldwork;
pub use log;
#[cfg(feature = "server")]
pub use schemars;
pub use serde;
pub use serde_json;
#[cfg(feature = "server")]
pub use server::{DEFAULT_TOOLS_TTL_MS, ServerConfig, handle_request};
#[cfg(any(feature = "cli", feature = "session"))]
pub use shellexpand;
#[cfg(feature = "stdio")]
pub use stdio::serve;
