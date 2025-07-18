use super::*;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_create_project_compiles() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let project_path = temp_dir.path().join("test-server");

    let opts = CreateOptions {
        name: "test-server",
        tools: &[
            "hello".to_string(),
            "greet".to_string(),
            "status".to_string(),
        ],
        state: "TestState",
        description: Some("A test MCP server"),
        instructions: Some("Test instructions for the server"),
    };

    // Create the project
    create_project(&opts, &project_path).expect("Failed to create project");

    // Verify basic structure exists
    assert!(project_path.join("Cargo.toml").exists());
    assert!(project_path.join("src/main.rs").exists());
    assert!(project_path.join("src/state.rs").exists());
    assert!(project_path.join("src/tools.rs").exists());
    assert!(project_path.join("src/tools/hello.rs").exists());
    assert!(project_path.join("src/tools/greet.rs").exists());
    assert!(project_path.join("src/tools/status.rs").exists());

    // Add a patch section to use the local mcplease
    let cargo_toml_path = project_path.join("Cargo.toml");
    let mut cargo_content =
        std::fs::read_to_string(&cargo_toml_path).expect("Failed to read Cargo.toml");

    // Find the mcplease source directory using the manifest dir
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mcplease_path = manifest_dir
        .parent()
        .expect("Failed to get parent directory");

    cargo_content = cargo_content.replace(
        "# [patch.crates-io]\n# mcplease = { path = \"../mcplease\" }",
        &format!(
            "[patch.crates-io]\nmcplease = {{ path = \"{}\" }}",
            mcplease_path.display()
        ),
    );

    std::fs::write(&cargo_toml_path, cargo_content).expect("Failed to write updated Cargo.toml");

    // Test that the generated project compiles
    let output = Command::new("cargo")
        .arg("check")
        .current_dir(&project_path)
        .output()
        .expect("Failed to run cargo check");

    if !output.status.success() {
        eprintln!("cargo check failed!");
        eprintln!("STDOUT: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("STDERR: {}", String::from_utf8_lossy(&output.stderr));
        panic!("Generated project does not compile");
    }

    println!("✅ Generated project compiles successfully!");
}

#[test]
fn test_cargo_toml_generation() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let project_path = temp_dir.path().join("toml-gen");

    let opts = CreateOptions {
        name: "my-test-server",
        tools: &[],
        state: "State",
        description: Some("Custom description"),
        instructions: None,
    };

    fs::create_dir_all(&project_path).expect("Failed to create project directory");
    generate_cargo_toml(&opts, &project_path).expect("Failed to generate Cargo.toml");

    let content =
        fs::read_to_string(project_path.join("Cargo.toml")).expect("Failed to read Cargo.toml");

    assert!(content.contains("name = \"my-test-server\""));
    assert!(content.contains("description = \"Custom description\""));
    assert!(content.contains("mcplease = \"0.2.0\""));
}

#[test]
fn test_tool_file_generation() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let project_path = temp_dir.path().join("tool-gen");
    fs::create_dir_all(project_path.join("src/tools")).expect("Failed to create directories");

    generate_tool_file("hello_world", "MyState", &project_path)
        .expect("Failed to generate tool file");

    let content = fs::read_to_string(project_path.join("src/tools/hello_world.rs"))
        .expect("Failed to read tool file");

    assert!(content.contains("pub struct HelloWorld"));
    assert!(content.contains("impl Tool<MyState> for HelloWorld"));
    assert!(content.contains("impl WithExamples for HelloWorld"));
    assert!(content.contains("#[serde(rename = \"hello_world\")]"));
}

#[test]
fn test_formatting_with_quote_newlines() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let project_path = temp_dir.path().join("test-formatting");

    // Create a project with multiple tools to test formatting
    let opts = CreateOptions {
        name: "test-formatting",
        tools: &["hello".to_string(), "goodbye".to_string()],
        state: "MyState",
        description: Some("Test formatting"),
        instructions: None,
    };

    create_project(&opts, &project_path).expect("Failed to create project");

    // Add another tool using absolute path
    add_tool_to_project_at_path("status", &project_path).expect("Failed to add tool");

    // Read the generated tools.rs and print it to see the formatting
    let tools_content =
        fs::read_to_string(project_path.join("src/tools.rs")).expect("Failed to read tools.rs");

    println!("Generated tools.rs content:\n{}", tools_content);

    // Verify it contains our tools
    assert!(tools_content.contains("Hello"));
    assert!(tools_content.contains("Goodbye"));
    assert!(tools_content.contains("Status"));
}

#[test]
fn test_add_tool_functionality() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let project_path = temp_dir.path().join("add-tool-test-unique");

    // First create a project
    let opts = CreateOptions {
        name: "test-project",
        tools: &["hello".to_string()],
        state: "MyState",
        description: Some("Test project"),
        instructions: None,
    };

    create_project(&opts, &project_path).expect("Failed to create project");

    // Add a new tool using absolute path (no working directory manipulation needed)
    add_tool_to_project_at_path("goodbye", &project_path).expect("Failed to add tool");

    // Verify the tool file was created
    assert!(project_path.join("src/tools/goodbye.rs").exists());

    // Verify tools.rs was updated
    let tools_content =
        fs::read_to_string(project_path.join("src/tools.rs")).expect("Failed to read tools.rs");

    assert!(tools_content.contains("Goodbye"));
    assert!(tools_content.contains("goodbye"));

    // Parse and verify the macro contains both tools
    let file: syn::File = syn::parse_str(&tools_content).expect("Failed to parse tools.rs");
    let tools_macro = find_tools_macro(&file).expect("No tools macro found");
    let args: ToolsMacroArgs =
        parse2(tools_macro.mac.tokens.clone()).expect("Failed to parse macro args");

    assert_eq!(args.tools.len(), 2);
    let tool_names: Vec<_> = args.tools.iter().map(|t| t.string_name.value()).collect();
    assert!(tool_names.contains(&"hello".to_string()));
    assert!(tool_names.contains(&"goodbye".to_string()));
}
