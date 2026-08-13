//! The command-line path: invoke a tool directly from argv, or start the stdio
//! server.
//!
//! Running a tool from a shell is how a tool is exercised without an MCP client
//! in the loop, which is why the `tools!` macro derives clap's [`Subcommand`]
//! for the generated `Tools` enum when this feature is on.

use crate::{
    server::ServerConfig,
    stdio::serve,
    traits::{AsToolsList, Dispatch},
    types::RequestContext,
};
use anyhow::Result;
use clap::{Parser, Subcommand};
use env_logger::{Builder, Target};
use std::{fmt::Debug, fs::OpenOptions, path::PathBuf};

#[derive(clap::Parser)]
struct Cli<T: Subcommand> {
    #[command(subcommand)]
    tool: T,
}

pub fn run<Tools: Debug + Subcommand + AsToolsList + Dispatch<State>, State>(
    state: &mut State,
    config: ServerConfig,
) -> Result<()> {
    if let Ok(log_location) = std::env::var("MCP_LOG_LOCATION") {
        let path = PathBuf::from(&*shellexpand::tilde(&log_location));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        Builder::from_default_env()
            .target(Target::Pipe(Box::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .unwrap(),
            )))
            .init();
    }

    match Cli::<Tools>::try_parse() {
        Ok(Cli { tool }) => {
            // No MCP client on this path, so there is no caller to describe.
            let result = tool.call_to_text(state, &RequestContext::default())?;
            println!("{result}");
        }
        Err(e) => {
            if std::env::args().nth(1).as_deref() == Some("serve") {
                serve::<Tools, State>(state, &config)?;
            } else {
                eprintln!("{e}");
            }
        }
    }

    Ok(())
}
