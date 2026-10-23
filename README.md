# MCPlease

MCPlease is a lightweight Rust framework for building MCP (Model Context Protocol) servers. It
provides a simple, macro-driven approach to defining tools, and tracks the `2026-07-28` revision of
the protocol.

## Features

- **Simple tool definition** using the `tools!` macro
- **Automatic JSON Schema generation** from Rust structs using `schemars`
- **Concurrent async dispatch** — calls run alongside each other, with progress
  notifications and cancellation, on whatever runtime your binary brings
- **Command-line interface** with automatic help generation via `clap`
- **Example system** for better tool documentation
- **Stdio transport** included; the protocol core does no I/O, so any transport can use it
- **Code generation CLI** for rapid development

## Bring your own runtime

Tools are `async fn`s, and this crate names no runtime and starts none: the binary brings one. The
stdio transport runs over any `futures-lite` `AsyncRead`/`AsyncWrite` pair — smol's `Unblock`, or
tokio's stdio through `async-compat` — and hands out each tool call as a self-contained future for
the binary to spawn. `mcplease create --runtime smol|tokio` writes that `main` for you.

## Quick Start with CLI (Recommended)

The fastest way to get started is with the mcplease CLI tool:

```bash
# Install the CLI
cargo install mcplease-cli

# Create a new MCP server with tools (`--runtime tokio` for tokio; smol is the default)
mcplease create my-server --tools hello,goodbye,status --state MyServerState

# Navigate to your project
cd my-server

# Add more tools as needed
mcplease add health_check
mcplease add ping

# Test that it compiles
cargo check

# Run your MCP server
cargo run serve
```

This creates a fully functional MCP server with:
- ✅ Proper project structure
- ✅ Generated tool implementations (with TODOs for you to fill in)
- ✅ State management boilerplate
- ✅ All necessary dependencies
- ✅ Beautifully formatted code

**For detailed CLI documentation, see [cli/README.md](./cli/README.md)**

## Manual Setup

### 1. Create a new MCP server project

```bash
cargo new my-mcp-server
cd my-mcp-server
```

### 2. Add dependencies to `Cargo.toml`

```toml
[dependencies]
anyhow = "1.0"
clap = { version = "4.5", features = ["derive"] }
env_logger = "0.11"
fieldwork = "0.4.6"
mcplease = "0.3.0"
schemars = "1.0.4"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
smol = "2"
```

### 3. Define your state structure

State is `Clone + Send + Sync + 'static` and every call gets its own clone, so it holds shared
handles rather than owned data. `Clone` rather than `Arc<State>` because the things you actually
hold — a pool, a client, a channel sender — are already internally shared.

It is *not* a place to keep per-caller state. `2026-07-28` removed protocol sessions and made every
request carry its own context, so anything that varies by caller belongs in the request: a tool
argument, or a token you hand the client and it hands back.

Create `src/state.rs`:

```rust
use anyhow::Result;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct MyToolsState {
    /// Read-only and shared: cloning is an `Arc` bump.
    config: Arc<Config>,
    /// Mutated by tools running at the same time, so it carries its own lock.
    cache: Arc<Mutex<Cache>>,
}

impl MyToolsState {
    pub fn new() -> Result<Self> {
        Ok(Self {
            config: Arc::new(Config::load()?),
            cache: Arc::default(),
        })
    }
}
```

### 4. Create tools

Create `src/tools/` directory and add tool implementations. Each tool should be in its own module:

**src/tools/hello.rs:**
```rust
use crate::state::MyToolsState;
use anyhow::Result;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext},
};
use serde::{Deserialize, Serialize};

/// Say hello to someone
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "hello")]
pub struct Hello {
    /// The name to greet
    pub name: String,
    
    /// Whether to be enthusiastic
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub enthusiastic: Option<bool>,
}

impl ToolMeta for Hello {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "A simple greeting",
                item: Self {
                    name: "World".into(),
                    enthusiastic: None,
                },
            },
            Example {
                description: "An enthusiastic greeting",
                item: Self {
                    name: "Alice".into(),
                    enthusiastic: Some(true),
                },
            },
        ]
    }
}

impl Tool<MyToolsState> for Hello {
    type Output = String;

    async fn execute(self, _state: MyToolsState, _context: RequestContext) -> Result<String> {
        let greeting = if self.enthusiastic.unwrap_or(false) {
            format!("Hello, {}! 🎉", self.name)
        } else {
            format!("Hello, {}", self.name)
        };
        Ok(greeting)
    }
}
```

**src/tools/count.rs** — a slow tool that reports progress and honors cancellation:
```rust
use crate::state::MyToolsState;
use anyhow::Result;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::RequestContext,
};
use serde::{Deserialize, Serialize};

/// Process a batch of items.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "count")]
pub struct Count {
    /// How many items to process.
    pub items: u32,
}

impl ToolMeta for Count {}

impl Tool<MyToolsState> for Count {
    type Output = String;

    async fn execute(self, _state: MyToolsState, context: RequestContext) -> Result<String> {
        let total = f64::from(self.items);
        for item in 1..=self.items {
            // Silent unless the client asked for progress by sending a
            // `progressToken`, and `?` here is how this tool stops when the
            // client cancels.
            context.progress(f64::from(item), Some(total), Some("processing"))?;
        }
        Ok(format!("processed {} items", self.items))
    }
}
```

A cancelled call's future is dropped at its next await point, so an async tool stops without
looking. Code with no await in it — like the loop above — cannot be interrupted, so it checks:
`progress` returning `Err(Cancelled)` is the usual checkpoint, [`RequestContext::is_cancelled`] is
there for a tool with nothing to report, and [`RequestContext::cancelled`] is a future for racing
work handed off elsewhere. Either way the transport sends no response for a cancelled call, which is
what the spec asks for.

### 5. Wire everything together

**src/tools.rs:**
```rust
use crate::state::MyToolsState;

mcplease::tools!(
    MyToolsState,
    (Hello, hello, "hello"),
    (Count, count, "count")
);
```

`Tools` derives clap's `Subcommand` so each tool can be run from a shell, which needs every
tool to derive `clap::Args`. A server with no command line — embedded, or behind another
transport — writes `mcplease::tools!(cli = false, MyToolsState, ...)` instead.

**src/main.rs:**
```rust
mod state;
mod tools;

use anyhow::Result;
use mcplease::{
    Cli, ServerConfig, clap::Parser, server_info, traits::Dispatch, types::RequestContext,
};
use smol::{Executor, Unblock};
use state::MyToolsState;
use tools::Tools;

const INSTRUCTIONS: &str = "This is my custom MCP server.";

fn main() -> Result<()> {
    // stderr is the one place a stdio server may log.
    env_logger::init();
    let state = MyToolsState::new()?;

    match Cli::<Tools>::parse() {
        Cli::Tool(tool) => {
            let output = smol::block_on(tool.call_to_text(state, RequestContext::default()))?;
            println!("{output}");
            Ok(())
        }

        Cli::Serve => {
            let config = ServerConfig::new(server_info!()).with_instructions(INSTRUCTIONS);
            let executor = Executor::new();
            smol::block_on(executor.run(async {
                let stdin = Unblock::new(std::io::stdin());
                let stdout = Unblock::new(std::io::stdout());
                let mut calls = mcplease::calls::<Tools, _>(stdin, stdout, state, &config);
                while let Some(call) = calls.next().await {
                    executor.spawn(call?).detach();
                }
                Ok(())
            }))
        }
    }
}
```

On tokio, the same loop reads `Compat::new(tokio::io::stdin())` (from `async-compat`) and
`tokio::spawn`s each call.

### 6. Run your server

```bash
# Run as MCP server (stdio mode)
cargo run serve

# Or use tools directly from command line
cargo run hello --name "World"
cargo run count --items 10
```

## Framework Architecture

### Core Components

1. **`tools!` macro**: Generates the enum that implements MCP tool dispatch
2. **`Tool` trait**: Defines how individual tools execute
3. **`ToolMeta` trait**: Examples, behavior hints, title, and icons for a tool
4. **`ToolOutput` trait**: What a tool returns — plain text or a structured value
5. **`RequestContext`**: Who called, plus progress reporting and cancellation
6. **JSON Schema generation**: Automatic from Rust structs via `schemars`

### Tool Definition Pattern

Each tool follows this pattern:

```rust
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "tool_name")]
pub struct MyTool {
    // Tool parameters with proper documentation
    /// Description of the parameter
    pub required_param: String,
    
    /// Optional parameter with skip_serializing_if
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub optional_param: Option<bool>,
}

impl ToolMeta for MyTool { /* ... */ }
impl Tool<StateType> for MyTool { /* ... */ }
```

### State Management

State is a bundle of shared handles, cloned per call. Anything mutable carries its own lock or
channel; the framework never hands out `&mut`.

```rust
#[derive(Clone)]
pub struct State {
    pool: sqlx::PgPool,          // already internally shared
    cache: Arc<Mutex<Cache>>,    // shared and mutable, so it locks
}
```

Earlier versions shipped a `SessionStore<T>` for per-session state on disk. It is gone: it was the
only I/O in a crate whose whole claim is that it does none, and it modeled a protocol session in a
revision that removed sessions. A server that still wants it can vendor the module — it was
self-contained — and pick its own filesystem story.

### Concurrency

Calls run concurrently, bounded by [`ServerConfig::with_max_concurrent_calls`]. A call arriving past
the limit is refused immediately with `server busy` (`-32000`) rather than queued.

Concurrency is what the protocol asks for: request ids and progress tokens must be unique across
*active* requests, cancellation exists to stop one call while others continue, and
`subscriptions/listen` is a request that never returns, which a serialized server would deadlock on.
Tools are expected to be safe to run alongside each other — which for a multi-step tool sequence
means keying its state by a token the client carries, rather than assuming it is the only one
running. Setting the limit to `1` restores single-file execution while you get there.

Every call is spawned, so tools share the executor with each other. A tool with a synchronous
stretch — heavy computation, `std::fs`, waiting on a subprocess — moves it to a blocking pool
(`smol::unblock`, `tokio::task::spawn_blocking`) rather than stalling its neighbours.

`calls(reader, writer, state, &config)` is the transport, and the loop in `main` above is the whole
driver. The stream answers `ping`, `tools/list` and friends itself, and hands out each admitted
`tools/call` as a `Send + 'static` future. It ends only once the input has closed *and* every call
has been answered, so a client that closes stdin and keeps reading gets every response. It yields
`Err` once if the output fails; dropping it cancels the calls still running.

Spawn every call. Awaiting each one before taking the next stops the stream — and with it pings,
cancellations, and every other call — for as long as a tool runs.

A tool that panics is answered with an internal error.

## Advanced Features

### Error Handling

Tools return `anyhow::Result<Self::Output>`. A returned `Err` becomes a tool
result with `isError: true` rather than a JSON-RPC protocol error, so the model
sees the failure and can correct itself:

```rust
impl Tool<State> for MyTool {
    type Output = String;

    async fn execute(self, state: State, _context: RequestContext) -> Result<String> {
        // Use ? for error propagation
        let data = std::fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read {}", self.path))?;
        
        // Return success message
        Ok(format!("Successfully processed {} bytes", data.len()))
    }
}
```

### Examples and Documentation

Provide meaningful examples to help users understand tool usage:

```rust
impl ToolMeta for MyTool {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Basic usage with default settings",
                item: Self {
                    path: "example.txt".into(),
                    options: None,
                },
            },
            Example {
                description: "Advanced usage with custom options",
                item: Self {
                    path: "/absolute/path/file.txt".into(),
                    options: Some(CustomOptions { verbose: true }),
                },
            },
        ]
    }
}
```

### Structured Output

A tool returning `String` sends plain text to the model and nothing else. A tool
can instead return a structured type, which is sent *both* as text for the model
and as a machine-readable value the calling client can display without parsing
the text. The type's JSON Schema is advertised as the tool's `outputSchema`, so
a client knows the shape before ever calling it.

```rust
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Weather {
    pub temperature: f64,
    pub conditions: String,
}

// Implements ToolOutput using the spec's recommended shape.
mcplease::structured_output!(Weather);

impl Tool<State> for GetWeather {
    type Output = Weather;

    async fn execute(self, state: State, _context: RequestContext) -> Result<Weather> {
        Ok(Weather { temperature: 22.5, conditions: "Partly cloudy".into() })
    }
}
```

By default the same value is sent both ways — the text half is its serialized
JSON — which is what keeps clients that ignore structured content working. For
large results, implement `ToolOutput` by hand and override `to_content` to send
the model a summary while the client still receives the full record.

### Rich Content

Returning `Vec<ContentBlock>` sends images, audio, embedded resources, or
several blocks at once. Blocks carry optional annotations, which are how you
mark one block for the user and another for the model:

```rust
impl Tool<State> for Render {
    type Output = Vec<ContentBlock>;

    async fn execute(self, state: State, _context: RequestContext) -> Result<Self::Output> {
        Ok(vec![
            ContentBlock::text("Rendered 3 charts."),
            ContentBlock::Image {
                data: base64_png,
                mime_type: "image/png".into(),
                annotations: Some(Annotations {
                    audience: Some(vec!["user".into()]),
                    ..Default::default()
                }),
                meta: None,
            },
        ])
    }
}
```

### Tool Annotations

Behavior hints tell a client what a tool does before it runs one, which is what
lets it decide what to auto-approve. The protocol's defaults are pessimistic —
an undeclared tool is assumed to be a destructive, open-world mutation — so
declaring them is worthwhile:

```rust
impl ToolMeta for ListFiles {
    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations {
            read_only_hint: Some(true),
            open_world_hint: Some(false),
            ..Default::default()
        })
    }

    fn title() -> Option<&'static str> {
        Some("List Files")
    }
}
```

### Knowing the Caller

`RequestContext` describes the client making *this* call — its protocol version,
self-reported identity, and the capabilities it supports. Capabilities are
declared per-request and must not be carried over from earlier ones, so this is
rebuilt for each call. On the command-line path it is empty, since there is no
MCP client.

```rust
async fn execute(self, state: State, context: RequestContext) -> Result<String> {
    if let Some(client) = &context.client_info {
        log::info!("called by {} {}", client.name, client.version);
    }
    // ...
}
```

### Server Configuration

`ServerConfig` carries everything the serve loop needs beyond the tools:

```rust
let config = ServerConfig::new(server_info!())
    .with_instructions(INSTRUCTIONS)
    // How long a client may cache the tool list. The list is fixed at compile
    // time, so the default is generous (one hour).
    .with_tools_ttl_ms(60 * 60 * 1000)
    // Opt in only if the tool list is identical for every user.
    .with_tools_cache_scope(CacheScope::Public)
    // How many tool calls may run at once. A call past this is refused at
    // once with `-32000 server busy` rather than queued. Default 16.
    .with_max_concurrent_calls(16);

let mut calls = mcplease::calls::<Tools, _>(stdin, stdout, state, &config);
```

Over stdio, `ping`, `initialize`, `server/discover`, and `tools/list` are answered by the stream
itself, in order with the input, so protocol traffic never waits on a tool. See
[Concurrency](#concurrency).

### Optional Parameters

Use `Option<T>` with proper serialization handling:

```rust
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
pub struct MyTool {
    /// Required parameter
    pub required: String,
    
    /// Optional parameter (won't appear in JSON if None)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub optional: Option<String>,
    
    /// Boolean flag (defaults to false)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub flag: Option<bool>,
}

impl MyTool {
    fn flag(&self) -> bool {
        self.flag.unwrap_or(false)
    }
}
```

### Sharing context between servers

Earlier versions did this through a shared `SessionStore` file. With sessions gone from the protocol
and `SessionStore` gone from this crate, the shape that survives a concurrent, session-free server
is to make the shared thing explicit: hold a handle in state, and let the client name what it wants
in the request.

```rust
#[derive(Clone)]
pub struct MyState {
    context: Arc<ContextService>,   // whatever backs it: a file, a socket, a daemon
}
```

## Best Practices

### Tool Design

1. **Single responsibility**: Each tool should do one thing well
2. **Clear documentation**: Use detailed doc comments on all parameters
3. **Meaningful examples**: Provide realistic usage scenarios
4. **Error context**: Use `anyhow::Context` for descriptive error messages
5. **Defensive programming**: Validate inputs and handle edge cases

### State Management

1. **Handles, not data**: State is cloned per call, so hold `Arc`s and pools
2. **Lock what you mutate**: Calls run concurrently; nothing hands you `&mut`
3. **Per-caller state goes in the request**: a tool argument or a token the client carries, not a
   server-side session
4. **Report progress**: anything that can take more than a moment should call
   `context.progress(..)`, which is also how it learns it was cancelled

### Error Messages

Return user-friendly messages that help with debugging:

```rust
// Good: Specific and actionable
Ok(format!("File {} does not exist. Use an absolute path or set_working_directory first.", path))

// Bad: Generic and unhelpful  
Err(anyhow!("File not found"))
```

### Path Handling

Use consistent path resolution patterns:

```rust
fn resolve_path(base: Option<&Path>, input: &str) -> Result<PathBuf> {
    let path = PathBuf::from(&*shellexpand::tilde(input));
    
    if path.is_absolute() {
        Ok(path)
    } else if let Some(base) = base {
        Ok(base.join(path))
    } else {
        Err(anyhow!("Relative path requires working directory to be set"))
    }
}
```

## Debugging

### Logging

A generated server logs to stderr through `env_logger` — the one place the stdio transport lets a
server write freely. Clients typically capture it.

```bash
RUST_LOG=debug cargo run serve
```

### Testing Tools Directly

Use the command-line interface for testing:

```bash
# Test individual tools
cargo run my-tool --param value

# Get help
cargo run help
cargo run my-tool --help
```

### Common Issues

1. **Schema validation errors**: Ensure all fields have proper serde attributes
2. **`State: Clone + Send + Sync` not satisfied**: hold `Arc`s and locks rather than owned mutable
   data; a `Box<dyn Fn>` field usually just needs `+ Sync` adding
3. **Tools interfering with each other**: they now run concurrently — key any multi-step sequence by
   a token the client carries, or set `with_max_concurrent_calls(1)` while you migrate
4. **A tool call hangs everything else**: a blocking tool stalls the calls it shares a task with;
   hand real blocking work to your runtime's offload
5. **JSON parsing**: Check that tool parameters match expected schema

## Contributing

When adding new tools to existing servers:

1. Create a new module in `src/tools/`
2. Implement the required traits
3. Add to the `tools!` macro in `src/tools.rs`
4. Add tests in `src/tests.rs`
5. Update documentation and examples

The framework is designed to be extensible - new MCP servers should follow the established patterns for consistency and maintainability.
