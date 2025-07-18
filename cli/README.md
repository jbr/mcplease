# MCPlease CLI

The MCPlease CLI is a code generation tool that makes building MCP (Model Context Protocol) servers fast and enjoyable. It handles all the boilerplate so you can focus on implementing your tool logic.

## Installation

```bash
cargo install mcplease-cli
```

## Commands

### `mcplease create`

Creates a new MCP server project with the specified tools.

```bash
mcplease create <PROJECT_NAME> --tools <TOOL1,TOOL2,...> [OPTIONS]
```

**Arguments:**
- `<PROJECT_NAME>` - Name of the project to create

**Options:**
- `--tools <TOOLS>` - Comma-separated list of tool names to generate
- `--state <STATE>` - Name of the state type (default: "State") 
- `--output <DIR>` - Output directory (default: same as project name)
- `--description <DESC>` - Project description for Cargo.toml
- `--instructions <TEXT>` - Instructions for the MCP server

**Examples:**

```bash
# Basic server with a few tools
mcplease create my-server --tools hello,goodbye,status

# Server with custom state type and description
mcplease create file-manager \
  --tools read,write,delete,list \
  --state FileManagerState \
  --description "A file management MCP server"

# Server with custom instructions
mcplease create calculator \
  --tools add,subtract,multiply,divide \
  --instructions "Use this server to perform basic arithmetic operations"
```

**Generated Structure:**
```
my-server/
├── Cargo.toml
└── src/
    ├── main.rs           # Entry point with server setup
    ├── state.rs          # State struct definition
    ├── tools.rs          # Tools macro invocation
    └── tools/
        ├── hello.rs      # Individual tool implementations
        ├── goodbye.rs
        └── status.rs
```

### `mcplease add`

Adds a new tool to an existing MCP server project.

```bash
mcplease add <TOOL_NAME>
```

**Arguments:**
- `<TOOL_NAME>` - Name of the tool to add

**Examples:**

```bash
# Add a single tool
mcplease add health_check

# Add multiple tools (run multiple times)
mcplease add ping
mcplease add version
mcplease add metrics
```

**What it does:**
1. ✅ Parses your existing `src/tools.rs` 
2. ✅ Adds the new tool to the `tools!` macro
3. ✅ Generates `src/tools/<tool_name>.rs` with boilerplate
4. ✅ Validates the tool doesn't already exist
5. ✅ Formats the code with `cargo fmt`
6. ✅ Preserves all your existing code

**Note:** Run this command from the root of your MCP server project (where `src/tools.rs` exists).

## Generated Code Structure

### Tool Implementation Template

Each generated tool follows this pattern:

```rust
use crate::state::MyState;
use anyhow::Result;
use mcplease::traits::{Tool, WithExamples};
use mcplease::types::Example;
use serde::{Deserialize, Serialize};

/// TODO: Add description for this tool
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "tool_name")]
pub struct ToolName {
    /// TODO: Add parameter description
    pub example_param: String,
}

impl WithExamples for ToolName {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "TODO: Add example description",
                item: Self {
                    example_param: "example_value".into(),
                },
            },
        ]
    }
}

impl Tool<MyState> for ToolName {
    fn execute(self, _state: &mut MyState) -> Result<String> {
        // TODO: Implement tool logic
        Ok(format!("tool_name executed with param: {}", self.example_param))
    }
}
```

### State Template

The generated state provides a foundation for session management:

```rust
use anyhow::Result;

/// State for the MCP server
///
/// TODO: Add your state fields here. Common patterns include:
/// - Working directory tracking
/// - Session management with mcplease::session::SessionStore
/// - Configuration data
/// - Cache or temporary data
#[derive(Debug)]
pub struct MyState {
    // TODO: Add your state fields here
}

impl MyState {
    pub fn new() -> Result<Self> {
        Ok(Self {
            // TODO: Initialize your state
        })
    }
}
```

### Tools Registration

The `tools.rs` file uses the `tools!` macro for clean registration:

```rust
use crate::state::MyState;

mcplease::tools!(
    MyState,
    (Hello, hello, "hello"),
    (Goodbye, goodbye, "goodbye"), 
    (Status, status, "status")
);
```

## Development Workflow

### 1. Create Your Server

```bash
mcplease create my-server --tools hello,status
cd my-server
```

### 2. Implement Your Tools

Edit the generated files in `src/tools/` to add your logic:

```rust
// src/tools/hello.rs
impl Tool<MyState> for Hello {
    fn execute(self, _state: &mut MyState) -> Result<String> {
        Ok(format!("Hello, {}!", self.name))
    }
}
```

### 3. Add State if Needed

```rust
// src/state.rs
use mcplease::session::SessionStore;

#[derive(Debug)]
pub struct MyState {
    session_store: SessionStore<SessionData>,
}
```

### 4. Test Your Server

```bash
# Check that everything compiles
cargo check

# Run the server
cargo run serve

# Test tools via command line
cargo run hello --name "World"
cargo run status
```

### 5. Add More Tools

```bash
mcplease add goodbye
mcplease add version
```

## Best Practices

### Tool Design
- **Keep tools focused** - each tool should do one thing well
- **Use descriptive names** - `get_file_content` vs `read`
- **Document parameters** - replace TODO comments with real descriptions
- **Provide good examples** - help users understand how to use your tools

### State Management
- **Use SessionStore** for persistent data that survives restarts
- **Keep state minimal** - only store what you actually need
- **Handle errors gracefully** - use `anyhow::Result` for clear error messages

### Code Organization
- **Keep tool files small** - complex logic can go in separate modules
- **Use descriptive parameter names** - `file_path` vs `path`
- **Add validation** - check inputs before processing

## Advanced Usage

### Custom State with Sessions

```rust
use mcplease::session::SessionStore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionData {
    pub working_directory: Option<PathBuf>,
    pub user_preferences: HashMap<String, String>,
}

#[derive(Debug)]
pub struct MyState {
    session_store: SessionStore<SessionData>,
}

impl MyState {
    pub fn new() -> Result<Self> {
        let session_store = SessionStore::new(Some(
            dirs::home_dir()
                .unwrap_or_default()
                .join(".my-server/sessions.json")
        ))?;
        Ok(Self { session_store })
    }
    
    pub fn get_working_directory(&mut self) -> Result<Option<PathBuf>> {
        Ok(self.session_store.get_or_create("default")?.working_directory.clone())
    }
}
```

### Complex Tool Parameters

```rust
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "search_files")]
pub struct SearchFiles {
    /// Pattern to search for (supports regex)
    pub pattern: String,
    
    /// Directory to search in
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub directory: Option<String>,
    
    /// File extensions to include
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, value_delimiter = ',')]
    pub extensions: Option<Vec<String>>,
    
    /// Whether to search recursively
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub recursive: Option<bool>,
}
```

### Rich Examples

```rust
impl WithExamples for SearchFiles {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Search for TODO comments in Rust files",
                item: Self {
                    pattern: "TODO".into(),
                    directory: Some("src".into()),
                    extensions: Some(vec!["rs".into()]),
                    recursive: Some(true),
                },
            },
            Example {
                description: "Find all JSON files in current directory", 
                item: Self {
                    pattern: r"\.json$".into(),
                    directory: None,
                    extensions: None,
                    recursive: Some(false),
                },
            },
        ]
    }
}
```

## Troubleshooting

### "No src/tools.rs found"
You're not in the root of an mcplease project. Navigate to the directory containing `src/tools.rs`.

### "Tool already exists"
The tool name you're trying to add already exists in the `tools!` macro. Use a different name or modify the existing tool.

### "cargo fmt failed"
The CLI will still generate your files, but they may not be perfectly formatted. This usually means:
- `cargo` is not in your PATH
- The generated code has syntax errors (shouldn't happen, but report as a bug)
- You're in a directory without a `Cargo.toml`

### Generated code doesn't compile
This shouldn't happen! If it does, please file an issue with:
- The command you ran
- The generated code
- The compilation error

## Contributing

The CLI is part of the mcplease workspace. To contribute:

```bash
git clone https://github.com/jbr/mcplease
cd mcplease

# Test the CLI
cargo test -p mcplease-cli

# Build the CLI
cargo build -p mcplease-cli

# Test code generation
cargo run -p mcplease-cli -- create test-server --tools hello,world
```

## Examples

### File Manager Server
```bash
mcplease create file-manager \
  --tools read,write,delete,list,move \
  --state FileManagerState \
  --description "MCP server for file operations"
```

### HTTP Client Server  
```bash
mcplease create http-client \
  --tools get,post,put,delete \
  --state HttpClientState \
  --instructions "Use this server to make HTTP requests"
```

### Database Server
```bash
mcplease create db-server \
  --tools query,insert,update,delete,migrate \
  --state DatabaseState

cd db-server
mcplease add backup
mcplease add restore
```

---

**Back to main documentation:** [../README.md](../README.md)
