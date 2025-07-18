use anyhow::{Context, Result};
use clap::Parser;
use heck::{ToPascalCase, ToSnakeCase};
use prettyplease;
use quote::{format_ident, quote};
use std::fs;
use std::path::PathBuf;
use syn::{
    File, Item, ItemImpl, ItemStruct, Token, parse::Parse, parse_quote, parse2,
    punctuated::Punctuated,
};

#[derive(Parser)]
#[command(name = "mcplease")]
#[command(about = "CLI tool for creating MCP servers")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Parser)]
enum Commands {
    /// Create a new MCP server project
    Create {
        /// Project name
        name: String,

        /// Tool names to generate
        #[arg(long, value_delimiter = ',')]
        tools: Vec<String>,

        /// State type name
        #[arg(long, default_value = "State")]
        state: String,

        /// Output directory
        #[arg(long)]
        output: Option<PathBuf>,

        /// Server description
        #[arg(long)]
        description: Option<String>,

        /// Instructions for the MCP server
        #[arg(long)]
        instructions: Option<String>,
    },
    /// Add a new tool to an existing project
    Add {
        /// Tool name to add
        #[arg(long)]
        tool: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Create {
            name,
            tools,
            state,
            output,
            description,
            instructions,
        } => {
            let output_dir = output.unwrap_or_else(|| PathBuf::from(&name));

            if output_dir.exists() {
                return Err(anyhow::anyhow!(
                    "Directory {} already exists",
                    output_dir.display()
                ));
            }

            create_project(
                &CreateOptions {
                    name: &name,
                    tools: &tools,
                    state: &state,
                    description: description.as_deref(),
                    instructions: instructions.as_deref(),
                },
                &output_dir,
            )?;

            println!("✅ Created MCP server project: {}", output_dir.display());
            println!("📁 Project structure:");
            println!("   {}/", name);
            println!("   ├── Cargo.toml");
            println!("   └── src/");
            println!("       ├── main.rs");
            println!("       ├── state.rs");
            println!("       ├── tools.rs");
            println!("       └── tools/");
            for tool in &tools {
                println!("           ├── {}.rs", tool.to_snake_case());
            }
            println!();
            println!("🚀 Next steps:");
            println!("   cd {}", name);
            println!("   cargo check  # Verify everything compiles");
            println!("   cargo run serve  # Start the MCP server");

            Ok(())
        }
        Commands::Add { tool } => {
            add_tool_to_project(&tool)?;
            Ok(())
        }
    }
}

// Custom parser for the tools! macro arguments
#[derive(Debug)]
struct ToolsMacroArgs {
    state_type: syn::Ident,
    tools: Punctuated<ToolEntry, Token![,]>,
}

#[derive(Clone, Debug)]
struct ToolEntry {
    struct_name: syn::Ident,
    mod_name: syn::Ident,
    string_name: syn::LitStr,
}

impl Parse for ToolsMacroArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let state_type = input.parse()?;
        input.parse::<Token![,]>()?;

        let tools = Punctuated::parse_terminated(input)?;

        Ok(ToolsMacroArgs { state_type, tools })
    }
}

impl Parse for ToolEntry {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        syn::parenthesized!(content in input);

        let struct_name = content.parse()?;
        content.parse::<Token![,]>()?;
        let mod_name = content.parse()?;
        content.parse::<Token![,]>()?;
        let string_name = content.parse()?;

        Ok(ToolEntry {
            struct_name,
            mod_name,
            string_name,
        })
    }
}

fn find_tools_macro(file: &syn::File) -> Option<&syn::ItemMacro> {
    file.items.iter().find_map(|item| {
        if let syn::Item::Macro(mac) = item {
            // Check for both "tools" and "mcplease::tools"
            if mac.mac.path.is_ident("tools")
                || (mac.mac.path.segments.len() == 2
                    && mac.mac.path.segments.last().unwrap().ident == "tools")
            {
                Some(mac)
            } else {
                None
            }
        } else {
            None
        }
    })
}

fn format_tools_file(project_path: &PathBuf) -> Result<()> {
    use std::process::Command;

    let output = Command::new("cargo")
        .arg("fmt")
        .arg("--")
        .arg("src/tools.rs")
        .current_dir(project_path)
        .output()
        .context("Failed to execute cargo fmt")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("cargo fmt failed: {}", stderr));
    }

    Ok(())
}

fn add_tool_to_project_impl(tool_name: &str, project_path: Option<&std::path::Path>) -> Result<()> {
    let base_path = project_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // 1. Check if we're in a project directory
    let tools_rs_path = base_path.join("src/tools.rs");
    if !tools_rs_path.exists() {
        return Err(anyhow::anyhow!(
            "No src/tools.rs found at {}. Run this command from the root of an mcplease project.",
            tools_rs_path.display()
        ));
    }

    // 2. Parse tools.rs
    let tools_content =
        fs::read_to_string(&tools_rs_path).context("Failed to read src/tools.rs")?;
    let file: syn::File = syn::parse_str(&tools_content).context("Failed to parse src/tools.rs")?;

    // 3. Find the tools! macro
    let tools_macro = find_tools_macro(&file)
        .ok_or_else(|| anyhow::anyhow!("No tools! macro found in src/tools.rs"))?;

    // 4. Parse the macro arguments
    let mut args: ToolsMacroArgs =
        parse2(tools_macro.mac.tokens.clone()).context("Failed to parse tools! macro arguments")?;

    // 5. Check if tool already exists
    let snake_name = tool_name.to_snake_case();
    if args
        .tools
        .iter()
        .any(|t| t.string_name.value() == snake_name)
    {
        return Err(anyhow::anyhow!("Tool '{}' already exists", tool_name));
    }

    // 6. Add the new tool
    let new_tool = ToolEntry {
        struct_name: format_ident!("{}", tool_name.to_pascal_case()),
        mod_name: format_ident!("{}", snake_name),
        string_name: syn::LitStr::new(&snake_name, proc_macro2::Span::call_site()),
    };
    args.tools.push(new_tool);

    // 7. Regenerate the file
    let new_file = regenerate_tools_file(&file, &args)?;
    let formatted = prettyplease::unparse(&new_file);
    fs::write(&tools_rs_path, formatted).context("Failed to write src/tools.rs")?;

    // 8. Format the file with cargo fmt for better macro formatting
    format_tools_file(&base_path).unwrap_or_else(|e| {
        eprintln!(
            "Warning: cargo fmt failed ({}), but file was generated successfully",
            e
        );
    });

    // 9. Generate the tool file
    generate_tool_file(tool_name, &args.state_type.to_string(), &base_path)?;

    println!("✅ Added tool '{}' to the project", tool_name);
    println!("📁 Generated: src/tools/{}.rs", snake_name);
    println!("🔧 Updated: src/tools.rs");

    Ok(())
}

fn add_tool_to_project(tool_name: &str) -> Result<()> {
    add_tool_to_project_impl(tool_name, None)
}

#[cfg(test)]
fn add_tool_to_project_at_path(tool_name: &str, project_path: &std::path::Path) -> Result<()> {
    add_tool_to_project_impl(tool_name, Some(project_path))
}

fn regenerate_tools_file(original: &syn::File, args: &ToolsMacroArgs) -> Result<syn::File> {
    let mut new_items = Vec::new();

    // Copy all non-macro items
    for item in &original.items {
        if !matches!(item, syn::Item::Macro(_mac) if find_tools_macro(&syn::File {
            shebang: None,
            attrs: vec![],
            items: vec![item.clone()],
        }).is_some())
        {
            new_items.push(item.clone());
        }
    }

    // Generate new tools! macro call using quote!
    let state_ident = &args.state_type;
    let tool_entries = args.tools.iter().map(|tool| {
        let struct_name = &tool.struct_name;
        let mod_name = &tool.mod_name;
        let string_name = &tool.string_name;
        quote! {
            (#struct_name, #mod_name, #string_name)
        }
    });

    let tools_macro_tokens = quote! {
        #state_ident,
        #(
            #tool_entries
        ),*
    };

    // Create the macro item
    let tools_macro_item = syn::Item::Macro(syn::ItemMacro {
        attrs: vec![],
        ident: None,
        mac: syn::Macro {
            path: parse_quote! { mcplease::tools },
            bang_token: Default::default(),
            delimiter: syn::MacroDelimiter::Paren(Default::default()),
            tokens: tools_macro_tokens,
        },
        semi_token: Some(Default::default()),
    });

    new_items.push(tools_macro_item);

    Ok(syn::File {
        shebang: original.shebang.clone(),
        attrs: original.attrs.clone(),
        items: new_items,
    })
}

pub struct CreateOptions<'a> {
    pub name: &'a str,
    pub tools: &'a [String],
    pub state: &'a str,
    pub description: Option<&'a str>,
    pub instructions: Option<&'a str>,
}

pub fn create_project(opts: &CreateOptions, output_dir: &PathBuf) -> Result<()> {
    // Create directory structure
    fs::create_dir_all(output_dir)?;
    fs::create_dir_all(output_dir.join("src"))?;
    fs::create_dir_all(output_dir.join("src/tools"))?;

    // Generate files
    generate_cargo_toml(opts, output_dir)?;
    generate_main_rs(opts, output_dir)?;
    generate_state_rs(opts, output_dir)?;
    generate_tools_rs(opts, output_dir)?;

    // Generate individual tool files
    for tool in opts.tools {
        generate_tool_file(tool, opts.state, output_dir)?;
    }

    Ok(())
}

fn generate_cargo_toml(opts: &CreateOptions, output_dir: &PathBuf) -> Result<()> {
    let description = opts
        .description
        .unwrap_or("An MCP server built with mcplease");

    let content = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
description = "{description}"

[dependencies]
anyhow = "1.0"
clap = {{ version = "4.5", features = ["derive"] }}
mcplease = "0.2.0"
schemars = "1.0.4"
serde = {{ version = "1.0", features = ["derive"] }}
serde_json = "1.0"

# Uncomment if you want to use the development version of mcplease
# [patch.crates-io]
# mcplease = {{ path = "../mcplease" }}
"#,
        name = opts.name,
        description = description
    );

    fs::write(output_dir.join("Cargo.toml"), content).context("Failed to write Cargo.toml")?;

    Ok(())
}

fn generate_main_rs(opts: &CreateOptions, output_dir: &PathBuf) -> Result<()> {
    let state_ident = format_ident!("{}", opts.state);
    let instructions = opts
        .instructions
        .unwrap_or("TODO: Add instructions for your MCP server");

    let file: File = parse_quote! {
        mod state;
        mod tools;

        use anyhow::Result;
        use mcplease::server_info;
        use state::#state_ident;

        const INSTRUCTIONS: &str = #instructions;

        fn main() -> Result<()> {
            let mut state = #state_ident::new()?;
            mcplease::run::<tools::Tools, _>(&mut state, server_info!(), Some(INSTRUCTIONS))
        }
    };

    let content = prettyplease::unparse(&file);
    fs::write(output_dir.join("src/main.rs"), content).context("Failed to write main.rs")?;

    Ok(())
}

fn generate_state_rs(opts: &CreateOptions, output_dir: &PathBuf) -> Result<()> {
    let state_ident = format_ident!("{}", opts.state);

    let file: File = parse_quote! {
        use anyhow::Result;

        /// State for the MCP server
        ///
        /// TODO: Add your state fields here. Common patterns include:
        /// - Working directory tracking
        /// - Session management with mcplease::session::SessionStore
        /// - Configuration data
        /// - Cache or temporary data
        #[derive(Debug)]
        pub struct #state_ident {
            // TODO: Add your state fields here
        }

        impl #state_ident {
            pub fn new() -> Result<Self> {
                Ok(Self {
                    // TODO: Initialize your state
                })
            }
        }
    };

    let content = prettyplease::unparse(&file);
    fs::write(output_dir.join("src/state.rs"), content).context("Failed to write state.rs")?;

    Ok(())
}

fn generate_tools_rs(opts: &CreateOptions, output_dir: &PathBuf) -> Result<()> {
    let state_ident = format_ident!("{}", opts.state);

    // Only generate the use statement for the state - the tools! macro handles mod declarations
    let items: Vec<Item> = vec![parse_quote! {
        use crate::state::#state_ident;
    }];

    // Create the macro call as a raw item since syn doesn't have a clean way to represent macro calls
    let tools_macro_string = format!(
        "mcplease::tools!(\n    {},\n{}\n);",
        opts.state,
        opts.tools
            .iter()
            .map(|tool| {
                format!(
                    "    ({}, {}, \"{}\")",
                    tool.to_pascal_case(),
                    tool.to_snake_case(),
                    tool.to_snake_case()
                )
            })
            .collect::<Vec<_>>()
            .join(",\n")
    );

    let file = File {
        shebang: None,
        attrs: vec![],
        items,
    };

    let mut content = prettyplease::unparse(&file);
    content.push_str("\n\n");
    content.push_str(&tools_macro_string);
    content.push('\n');

    fs::write(output_dir.join("src/tools.rs"), content).context("Failed to write tools.rs")?;

    // Format the file with cargo fmt for better macro formatting
    format_tools_file(output_dir).unwrap_or_else(|e| {
        eprintln!(
            "Warning: cargo fmt failed ({}), but file was generated successfully",
            e
        );
    });

    Ok(())
}

fn generate_tool_file(tool_name: &str, state_name: &str, output_dir: &PathBuf) -> Result<()> {
    let tool_ident = format_ident!("{}", tool_name.to_pascal_case());
    let state_ident = format_ident!("{}", state_name);
    let snake_name = tool_name.to_snake_case();

    let tool_struct: ItemStruct = parse_quote! {
        /// TODO: Add description for this tool
        #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
        #[serde(rename = #snake_name)]
        pub struct #tool_ident {
            /// TODO: Add parameter description
            pub example_param: String,
        }
    };

    let examples_impl: ItemImpl = parse_quote! {
        impl WithExamples for #tool_ident {
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
    };

    let tool_impl: ItemImpl = parse_quote! {
        impl Tool<#state_ident> for #tool_ident {
            fn execute(self, _state: &mut #state_ident) -> Result<String> {
                // TODO: Implement tool logic
                Ok(format!("{} executed with param: {}", #snake_name, self.example_param))
            }
        }
    };

    let file = File {
        shebang: None,
        attrs: vec![],
        items: vec![
            // Use statements
            parse_quote! { use crate::state::#state_ident; },
            parse_quote! { use anyhow::Result; },
            parse_quote! { use mcplease::traits::{Tool, WithExamples}; },
            parse_quote! { use mcplease::types::Example; },
            parse_quote! { use serde::{Deserialize, Serialize}; },
            // Actual items
            tool_struct.into(),
            examples_impl.into(),
            tool_impl.into(),
        ],
    };

    let content = prettyplease::unparse(&file);
    let filename = format!("{}.rs", snake_name);
    fs::write(output_dir.join("src/tools").join(filename), content)
        .with_context(|| format!("Failed to write tool file for {}", tool_name))?;

    Ok(())
}

#[cfg(test)]
mod tests;
