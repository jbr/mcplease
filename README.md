# MCPlease

MCPlease is a lightweight Rust framework for building MCP (Model Context Protocol) servers. It provides a simple, macro-driven approach to defining tools and managing state, with built-in support for session persistence and cross-process synchronization.

## Features

- **Simple tool definition** using the `tools!` macro
- **Automatic JSON Schema generation** from Rust structs using `schemars`
- **Session management** with cross-process synchronization via file watching
- **Command-line interface** with automatic help generation via `clap`
- **Example system** for better tool documentation
- **Stdio-based MCP communication** (WebSocket support planned)
- **Code generation CLI** for rapid development

## Quick Start with CLI (Recommended)

The fastest way to get started is with the mcplease CLI tool:

```bash
# Install the CLI
cargo install mcplease-cli

# Create a new MCP server with tools
mcplease create my-server --tools hello,goodbye,status --state MyServerState

# Navigate to your project
cd my-server

# Add more tools as needed
mcplease add --tool health_check
mcplease add --tool ping

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
fieldwork = "0.4.6"
mcplease = "0.2.0"
schemars = "1.0.4"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

### 3. Define your state structure

Create `src/state.rs`:

```rust
use anyhow::Result;
use mcplease::session::SessionStore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SharedData {
    pub working_directory: Option<PathBuf>,
    // Add other shared state fields here
}

#[derive(Debug, fieldwork::Fieldwork)]
pub struct MyToolsState {
    #[fieldwork(get, get_mut)]
    session_store: SessionStore<SharedData>,
}

impl MyToolsState {
    pub fn new() -> Result<Self> {
        let session_store = SessionStore::new(Some(
            dirs::home_dir()
                .unwrap_or_default()
                .join(".ai-tools/sessions/my-tools.json")
        ))?;
        
        Ok(Self { session_store })
    }
    
    pub fn get_working_directory(&mut self) -> Result<Option<PathBuf>> {
        Ok(self.session_store.get_or_create("default")?.working_directory.clone())
    }
    
    pub fn set_working_directory(&mut self, path: PathBuf) -> Result<()> {
        self.session_store.update("default", |data| {
            data.working_directory = Some(path);
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
    traits::{Tool, WithExamples},
    types::Example,
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

impl WithExamples for Hello {
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
    fn execute(self, _state: &mut MyToolsState) -> Result<String> {
        let greeting = if self.enthusiastic.unwrap_or(false) {
            format!("Hello, {}! 🎉", self.name)
        } else {
            format!("Hello, {}", self.name)
        };
        Ok(greeting)
    }
}
```

**src/tools/set_working_directory.rs:**
```rust
use crate::state::MyToolsState;
use anyhow::Result;
use mcplease::{
    traits::{Tool, WithExamples},
    types::Example,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Set the working directory for relative path operations
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "set_working_directory")]
pub struct SetWorkingDirectory {
    /// New working directory path
    pub path: String,
}

impl WithExamples for SetWorkingDirectory {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Set working directory to a project folder",
                item: Self {
                    path: "/path/to/my/project".into(),
                },
            },
        ]
    }
}

impl Tool<MyToolsState> for SetWorkingDirectory {
    fn execute(self, state: &mut MyToolsState) -> Result<String> {
        let path = PathBuf::from(&*shellexpand::tilde(&self.path));
        
        if !path.exists() {
            return Ok(format!("Directory {} does not exist", path.display()));
        }
        
        state.set_working_directory(path.clone())?;
        Ok(format!("Set working directory to {}", path.display()))
    }
}
```

### 5. Wire everything together

**src/tools.rs:**
```rust
use crate::state::MyToolsState;

mcplease::tools!(
    MyToolsState,
    (Hello, hello, "hello"),
    (SetWorkingDirectory, set_working_directory, "set_working_directory")
);
```

**src/main.rs:**
```rust
mod state;
mod tools;

use anyhow::Result;
use mcplease::server_info;
use state::MyToolsState;

const INSTRUCTIONS: &str = "This is my custom MCP server. Use set_working_directory to establish context.";

fn main() -> Result<()> {
    let mut state = MyToolsState::new()?;
    mcplease::run::<tools::Tools, _>(&mut state, server_info!(), Some(INSTRUCTIONS))
}
```

### 6. Run your server

```bash
# Run as MCP server (stdio mode)
cargo run serve

# Or use tools directly from command line
cargo run hello --name "World"
cargo run set-working-directory --path "/tmp"
```

## Framework Architecture

### Core Components

1. **`tools!` macro**: Generates the enum that implements MCP tool dispatch
2. **`Tool` trait**: Defines how individual tools execute
3. **`WithExamples` trait**: Provides example usage for documentation
4. **`SessionStore`**: Handles persistent state with cross-process sync
5. **JSON Schema generation**: Automatic from Rust structs via `schemars`

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

impl WithExamples for MyTool { /* ... */ }
impl Tool<StateType> for MyTool { /* ... */ }
```

### State Management

The framework uses `SessionStore<T>` for persistent state:

- **Cross-process safe**: File watching detects external changes
- **Atomic writes**: Temporary file + rename prevents corruption
- **Session-based**: Multiple sessions can coexist
- **JSON serialization**: Human-readable storage format

### Session Store API

```rust
// Get or create session data
let data = store.get_or_create("session_id")?;

// Update data with closure
store.update("session_id", |data| {
    data.some_field = new_value;
})?;

// Get without creating
let maybe_data = store.get("session_id")?;

// Set directly
store.set("session_id", new_data)?;
```

## Example MCP Servers

The framework includes several reference implementations:

### fs-mcp (Filesystem Operations)
- **Tools**: read, write, delete, move, list, search, set_working_directory
- **Features**: Glob patterns, metadata, recursive operations
- **Session data**: Working directory context

### cargo-mcp (Rust Project Management)
- **Tools**: build, test, check, clippy, add/remove deps, clean, bench
- **Features**: Toolchain selection, package targeting, environment variables
- **Session data**: Project directory

### semantic-edit-mcp (Code Editing)
- **Tools**: preview_edit, retarget_edit, persist_edit, set_working_directory  
- **Features**: AST-aware editing, language detection, diff preview
- **Session data**: Staged operations, working directory

### rustdoc-json-mcp (Documentation)
- **Tools**: get_item, set_working_directory
- **Features**: Rustdoc JSON parsing, type information, source code
- **Session data**: Project manifest directory

## Advanced Features

### Error Handling

Tools should return `anyhow::Result<String>` for consistent error propagation:

```rust
impl Tool<State> for MyTool {
    fn execute(self, state: &mut State) -> Result<String> {
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
impl WithExamples for MyTool {
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

### Shared Session Data

For tools that need to share context across processes:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SharedContext {
    pub working_directory: Option<PathBuf>,
    pub recent_files: Vec<PathBuf>,
    pub user_preferences: HashMap<String, String>,
}

// In your state struct:
impl MyState {
    pub fn new() -> Result<Self> {
        // Use a shared file for cross-server communication
        let shared_store = SessionStore::new(Some(
            dirs::home_dir()
                .unwrap_or_default()
                .join(".ai-tools/sessions/shared-context.json")
        ))?;
        
        Ok(Self { shared_store })
    }
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

1. **Minimal state**: Only persist what's necessary across calls
2. **Default values**: Use `#[serde(default)]` for backward compatibility
3. **Session IDs**: Use logical identifiers like "default", project names, etc.
4. **Cleanup**: Consider implementing state cleanup for old sessions

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

Set `MCP_LOG_LOCATION` environment variable to enable logging:

```bash
export MCP_LOG_LOCATION="~/.ai-tools/logs/my-server.log"
cargo run serve
```

Log levels: `RUST_LOG=trace,warn,error,debug,info`

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
2. **Session conflicts**: Use unique session IDs for different contexts  
3. **Path resolution**: Always handle both absolute and relative paths
4. **JSON parsing**: Check that tool parameters match expected schema

## Contributing

When adding new tools to existing servers:

1. Create a new module in `src/tools/`
2. Implement the required traits
3. Add to the `tools!` macro in `src/tools.rs`
4. Add tests in `src/tests.rs`
5. Update documentation and examples

The framework is designed to be extensible - new MCP servers should follow the established patterns for consistency and maintainability.
