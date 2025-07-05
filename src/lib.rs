#[macro_use]
mod macros;
pub mod session;
pub mod traits;
pub mod types;

pub use anyhow;
pub use clap;
use clap::{Parser, Subcommand};
pub use serde;
pub use serde_json;

use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};

use anyhow::Result;
use env_logger::{Builder, Target};
use types::McpMessage;

use crate::{
    traits::{AsToolsList, Tool},
    types::Info,
};

fn serve<Tools: AsToolsList + Tool<State>, State>(
    state: &mut State,
    server_info: Info,
    instructions: Option<&'static str>,
) -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    log::trace!("started!");

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                log::trace!("<- {line}");
                match serde_json::from_str(&line) {
                    Ok(McpMessage::Request(request)) => {
                        let response =
                            request.execute::<State, Tools>(state, instructions, &server_info);
                        let response_str = serde_json::to_string(&response)?;
                        log::trace!("-> {response_str}");
                        stdout.write_all(response_str.as_bytes())?;
                        stdout.write_all(b"\n")?;
                        stdout.flush()?;
                    }
                    Ok(McpMessage::Notification(n)) => {
                        log::trace!("received {n:?}, ignoring");
                    }

                    Err(e) => {
                        log::error!("{e:?}");
                    }
                }
            }
            Err(e) => {
                log::error!("Error reading line: {e}");
                break;
            }
        }
    }

    Ok(())
}

#[derive(clap::Parser)]
struct Cli<T: Subcommand> {
    #[command(subcommand)]
    tool: T,
}

pub fn run<Tools: Subcommand + AsToolsList + Tool<State>, State>(
    state: &mut State,
    server_info: Info,
    instructions: Option<&'static str>,
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
            let result = tool.execute(state)?;
            println!("{result}");
        }
        Err(e) => {
            if std::env::args().nth(1).as_deref() == Some("serve") {
                serve::<Tools, State>(state, server_info, instructions)?;
            } else {
                println!("{e}");
            }
        }
    }

    Ok(())
}
