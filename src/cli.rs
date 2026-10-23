//! The command-line path: invoke a tool directly from argv, or serve.
//!
//! Running a tool from a shell is how a tool is exercised without an MCP client
//! in the loop, which is why the `tools!` macro derives clap's [`Subcommand`]
//! for the generated `Tools` enum when this feature is on.

use clap::{Parser, Subcommand};

/// What argv asks for: serve MCP on stdio, or run one tool and exit.
///
/// This decides and nothing more; the binary owns the runtime, so it does the
/// running:
///
/// ```ignore
/// match mcplease::Cli::<Tools>::parse() {
///     Cli::Serve => { /* drive `mcplease::calls` over stdin and stdout */ }
///     Cli::Tool(tool) => println!("{}", tool.call_to_text(state, RequestContext::default()).await?),
/// }
/// ```
#[derive(Debug, Parser)]
// Keep this rustdoc out of the binary's `--help`.
#[command(about = None, long_about = None)]
pub enum Cli<Tools: Subcommand> {
    /// Serve MCP over stdin and stdout
    Serve,

    #[command(flatten)]
    Tool(Tools),
}
