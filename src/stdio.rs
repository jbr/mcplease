//! The stdio transport: newline-delimited JSON-RPC over stdin and stdout.

use crate::{
    server::{ServerConfig, handle_request},
    traits::{AsToolsList, Dispatch},
    types::JsonRpcMessage,
};
use anyhow::Result;
use std::{
    fmt::Debug,
    io::{BufRead, BufReader, Write},
};

/// Read messages from stdin until EOF, answering each request on stdout.
///
/// Blocks the calling thread for the lifetime of the connection.
pub fn serve<Tools: Debug + AsToolsList + Dispatch<State>, State>(
    state: &mut State,
    config: &ServerConfig,
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
                if line.trim().is_empty() {
                    continue;
                }
                log::trace!("<- {line}");
                match serde_json::from_str(&line) {
                    Ok(JsonRpcMessage::Request(request)) => {
                        let response = handle_request::<Tools, State>(request, state, config);
                        let response_str = serde_json::to_string(&response)?;
                        log::trace!("-> {response_str}");
                        stdout.write_all(response_str.as_bytes())?;
                        stdout.write_all(b"\n")?;
                        stdout.flush()?;
                    }
                    Ok(message) => {
                        log::trace!("received {message:?}, ignoring");
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
