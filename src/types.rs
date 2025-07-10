use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{borrow::Cow, collections::HashMap};

use crate::traits::{AsToolsList, Tool};

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpMessage {
    #[serde(deserialize_with = "deserialize_request")]
    Request(McpRequest),
    Notification(McpNotification),
}

fn deserialize_request<'de, D>(deserializer: D) -> Result<McpRequest, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Value = Deserialize::deserialize(deserializer)?;
    if value.get("id").is_some() {
        // Use from_value instead of deserialize
        serde_json::from_value(value).map_err(serde::de::Error::custom)
    } else {
        Err(serde::de::Error::custom("Not a request"))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    pub params: Option<Value>,
}

impl McpRequest {
    pub fn execute<State, Tools: AsToolsList + Tool<State>>(
        self,
        state: &mut State,
        instructions: Option<&'static str>,
        server_info: &Info,
    ) -> McpResponse {
        let Self {
            id, method, params, ..
        } = self;
        match method.as_str() {
            "initialize" => McpResponse::success(
                id,
                InitializeResponse::new(server_info.to_owned()).with_instructions(instructions),
            ),
            "tools/list" => {
                let tools = Tools::tools_list();
                McpResponse::success(id, ToolsListResponse { tools })
            }
            "tools/call" => match serde_json::from_value::<Tools>(params.unwrap_or(Value::Null)) {
                Ok(tool) => match tool.execute(state) {
                    Ok(string) => {
                        log::debug!("{string}");
                        McpResponse::success(id, ContentResponse::text(string))
                    }
                    Err(e) => {
                        log::error!("{e}");
                        McpResponse::error(id, e.to_string())
                    }
                },
                Err(e) => {
                    log::error!("{e}");
                    McpResponse::error(id, e.to_string())
                }
            },
            _ => McpResponse::error(id, format!("Unknown method: {method}")),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    capabilities: Value,
    client_info: Info,
    protocol_version: String,
}

#[derive(Debug, Serialize, Deserialize, fieldwork::Fieldwork)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    protocol_version: &'static str,
    capabilities: Capabilities,
    server_info: Info,
    #[fieldwork(with)]
    instructions: Option<&'static str>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Example<T> {
    pub description: &'static str,
    #[serde(flatten)]
    pub item: T,
}

impl InitializeResponse {
    pub fn new(server_info: Info) -> Self {
        Self {
            protocol_version: "2024-11-05",
            capabilities: Capabilities::default(),
            server_info,
            instructions: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Info {
    pub name: Cow<'static, str>,
    pub version: Cow<'static, str>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub tools: HashMap<(), ()>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ToolsListResponse {
    pub tools: Vec<ToolSchema>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSchema {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: InputSchema,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputSchema {
    // Union types (check these first)
    AnyOf {
        #[serde(rename = "anyOf")]
        any_of: Vec<InputSchema>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    OneOf {
        #[serde(rename = "oneOf")]
        one_of: Vec<InputSchema>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        examples: Option<Vec<Value>>,
    },
    Tagged(Tagged),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Tagged {
    #[serde(rename = "object")]
    Object {
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        properties: HashMap<String, Box<InputSchema>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        required: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_properties: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        examples: Option<Vec<Value>>,
    },
    #[serde(rename = "string")]
    String {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        r#enum: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        examples: Option<Vec<String>>,
    },

    #[serde(rename = "boolean")]
    Boolean {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },

    #[serde(rename = "integer")]
    Integer {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },

    #[serde(rename = "array")]
    Array {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        items: Box<InputSchema>,
    },

    #[serde(rename = "null")]
    Null,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct ContentResponse {
    content: Vec<TextContent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TextContent {
    pub r#type: &'static str,
    pub text: String,
}

impl ContentResponse {
    pub fn text(text: String) -> Self {
        Self {
            content: vec![TextContent {
                r#type: "text",
                text,
            }],
        }
    }
}

impl McpResponse {
    pub fn success(id: Value, result: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(serde_json::to_value(result).unwrap()),
            error: None,
        }
    }

    pub fn error(id: Value, message: String) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(McpError {
                code: -32601,
                message,
                data: None,
            }),
        }
    }
}
