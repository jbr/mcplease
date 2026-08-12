#[macro_use]
mod macros;
pub mod session;
pub mod traits;
pub mod types;

pub use anyhow;
pub use clap;
pub use dirs;
pub use fieldwork;
pub use log;
pub use schemars;
pub use serde;
pub use serde_json;
pub use shellexpand;

use std::{
    fmt::Debug,
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};

use crate::{
    traits::{AsToolsList, Tool},
    types::{
        CacheScope, CallToolResult, DiscoverResult, Implementation, InitializeRequestParams,
        InitializeResult, JsonRpcError, JsonRpcMessage, JsonRpcRequest, JsonRpcResponse,
        ListToolsResult, ServerCapabilities, LATEST_HANDSHAKE_PROTOCOL_VERSION,
        SUPPORTED_PROTOCOL_VERSIONS,
    },
};
use anyhow::Result;
use clap::{Parser, Subcommand};
use env_logger::{Builder, Target};
use serde_json::json;

/// Answer one request. Dispatch is stateless: every supported revision's
/// entry points are answerable at any time, so both the `initialize`
/// handshake (revisions through 2025-11-25) and the stateless
/// `server/discover` flow (2026-07-28) work against the same loop.
fn handle_request<Tools: Debug + AsToolsList + Tool<State>, State>(
    request: JsonRpcRequest,
    state: &mut State,
    instructions: Option<&'static str>,
    server_info: &Implementation,
) -> JsonRpcResponse {
    let JsonRpcRequest {
        id, method, params, ..
    } = request;
    match method.as_str() {
        "initialize" => {
            // Echo a supported requested version; otherwise answer with the
            // newest handshake revision and let the client decide.
            let requested = params
                .and_then(|params| {
                    serde_json::from_value::<InitializeRequestParams>(params).ok()
                })
                .map(|params| params.protocol_version);
            let protocol_version = match requested {
                Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v.as_str()) => v,
                _ => LATEST_HANDSHAKE_PROTOCOL_VERSION.to_string(),
            };
            JsonRpcResponse::success(
                id,
                InitializeResult {
                    protocol_version,
                    capabilities: ServerCapabilities::tools_only(),
                    server_info: server_info.clone(),
                    instructions: instructions.map(String::from),
                    meta: None,
                },
            )
        }
        "server/discover" => JsonRpcResponse::success(
            id,
            DiscoverResult {
                supported_versions: SUPPORTED_PROTOCOL_VERSIONS
                    .iter()
                    .map(|v| v.to_string())
                    .collect(),
                capabilities: ServerCapabilities::tools_only(),
                instructions: instructions.map(String::from),
                ttl_ms: Some(0),
                cache_scope: Some(CacheScope::Private),
                result_type: Some("complete".into()),
                meta: Some(
                    json!({ "io.modelcontextprotocol/serverInfo": server_info })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            },
        ),
        "ping" => JsonRpcResponse::success(id, json!({})),
        "tools/list" => JsonRpcResponse::success(
            id,
            ListToolsResult {
                tools: Tools::tools_list(),
                ttl_ms: Some(0),
                cache_scope: Some(CacheScope::Private),
                result_type: Some("complete".into()),
                ..ListToolsResult::default()
            },
        ),
        "tools/call" => {
            match serde_json::from_value::<Tools>(params.unwrap_or(serde_json::Value::Null)) {
                Ok(tool) => {
                    log::info!("{tool:?}");
                    match tool.execute(state) {
                        Ok(text) => {
                            log::debug!("{text}");
                            JsonRpcResponse::success(id, CallToolResult::text(text))
                        }
                        // A failure of the tool itself is an is_error result,
                        // not a protocol error, so the model sees it.
                        Err(e) => {
                            log::error!("{e}");
                            JsonRpcResponse::success(id, CallToolResult::error(e.to_string()))
                        }
                    }
                }
                Err(e) => {
                    log::error!("{e}");
                    JsonRpcResponse::error(id, JsonRpcError::invalid_params(e.to_string()))
                }
            }
        }
        _ => JsonRpcResponse::error(id, JsonRpcError::method_not_found(&method)),
    }
}

fn serve<Tools: Debug + AsToolsList + Tool<State>, State>(
    state: &mut State,
    server_info: Implementation,
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
                if line.trim().is_empty() {
                    continue;
                }
                log::trace!("<- {line}");
                match serde_json::from_str(&line) {
                    Ok(JsonRpcMessage::Request(request)) => {
                        let response = handle_request::<Tools, State>(
                            request,
                            state,
                            instructions,
                            &server_info,
                        );
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

#[derive(clap::Parser)]
struct Cli<T: Subcommand> {
    #[command(subcommand)]
    tool: T,
}

pub fn run<Tools: Debug + Subcommand + AsToolsList + Tool<State>, State>(
    state: &mut State,
    server_info: Implementation,
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
                eprintln!("{e}");
            }
        }
    }

    Ok(())
}
