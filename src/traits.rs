use crate::types::{
    CallToolResult, ContentBlock, Example, Icon, RequestContext, Tool as ToolDefinition,
    ToolAnnotations,
};
use anyhow::Result;
use schemars::{
    JsonSchema, Schema,
    generate::SchemaSettings,
    transform::{RecursiveTransform, Transform},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Declarative metadata attached to a tool's advertised definition.
///
/// Every method has a default, so `impl ToolMeta for MyTool {}` is enough.
///
/// The behavior hints in [`annotations`](ToolMeta::annotations) are worth
/// setting: the spec's defaults are pessimistic — `destructiveHint` and
/// `openWorldHint` both default to `true` — so an undeclared tool is presumed
/// to be a destructive, open-world mutation. Note that clients are directed
/// not to make tool-use decisions on annotations from untrusted servers, so
/// these inform display and approval UX rather than authorization.
pub trait ToolMeta: Sized + Serialize {
    /// Worked examples, attached to the input schema's `examples` keyword.
    fn examples() -> Vec<Example<Self>> {
        vec![]
    }

    /// Behavior hints: read-only, destructive, idempotent, open-world.
    fn annotations() -> Option<ToolAnnotations> {
        None
    }

    /// A human-readable display name. Display precedence is `title`, then
    /// `annotations.title`, then the tool's `name`.
    fn title() -> Option<&'static str> {
        None
    }

    /// Icons for display in a user interface.
    fn icons() -> Option<Vec<Icon>> {
        None
    }
}

#[deprecated(since = "0.3.0", note = "renamed to ToolMeta")]
pub use ToolMeta as WithExamples;

/// What a tool returns.
///
/// The two halves address different audiences: [`to_content`](ToolOutput::to_content)
/// is what the model reads, and [`structured_content`](ToolOutput::structured_content)
/// is what the calling program reads, validated against
/// [`output_schema`](ToolOutput::output_schema).
///
/// The defaults implement the spec's recommended shape: the same value in
/// both, with the content half being its serialized JSON. That duplicates the
/// payload, which is deliberate — it is what keeps a client that ignores
/// `structuredContent` working. A tool with large results may override
/// `to_content` to return a summary instead; that is a deliberate deviation
/// from a SHOULD, and costs any such client the detail.
///
/// Implemented here for [`String`] (plain text, no structured half) and
/// `Vec<ContentBlock>` (multimodal content, no structured half). For a
/// structured type, [`structured_output!`](crate::structured_output) writes
/// the impl.
pub trait ToolOutput: Serialize {
    /// JSON Schema for [`structured_content`](ToolOutput::structured_content).
    /// When present, the spec requires the structured result to conform.
    fn output_schema() -> Option<Value> {
        None
    }

    /// The model-facing half.
    fn to_content(&self) -> Vec<ContentBlock> {
        vec![ContentBlock::text(
            serde_json::to_string(self).unwrap_or_default(),
        )]
    }

    /// The program-facing half.
    fn structured_content(&self) -> Option<Value> {
        serde_json::to_value(self).ok()
    }

    /// A plain-text rendering, used for the command-line path where there is
    /// no MCP client to hand content blocks to.
    fn to_text(&self) -> String {
        self.to_content()
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl ToolOutput for String {
    fn to_content(&self) -> Vec<ContentBlock> {
        vec![ContentBlock::text(self)]
    }

    fn structured_content(&self) -> Option<Value> {
        None
    }

    fn to_text(&self) -> String {
        self.clone()
    }
}

impl ToolOutput for Vec<ContentBlock> {
    fn to_content(&self) -> Vec<ContentBlock> {
        self.clone()
    }

    fn structured_content(&self) -> Option<Value> {
        None
    }
}

/// Generate the JSON Schema this crate advertises for `T`: inlined
/// subschemas, no meta-schema, and nullable unions flattened.
pub fn schema_for<T: JsonSchema>() -> Schema {
    let settings = SchemaSettings::draft2020_12().with(|settings| {
        settings.meta_schema = None;
        settings.inline_subschemas = true;
    });

    let mut schema = settings.into_generator().into_root_schema_for::<T>();
    RecursiveTransform(remove_null).transform(&mut schema);
    schema.remove("$schema");
    schema
}

/// Implement [`ToolOutput`] for types that derive [`JsonSchema`] and
/// [`Serialize`], using the spec's recommended lossless shape.
///
/// To emit something other than the serialized JSON to the model, write the
/// impl by hand and override `to_content`.
#[macro_export]
macro_rules! structured_output {
    ($($type:ty),+ $(,)?) => {
        $(
            impl $crate::traits::ToolOutput for $type {
                fn output_schema() -> Option<$crate::serde_json::Value> {
                    Some($crate::traits::schema_for::<Self>().into())
                }
            }
        )+
    };
}

fn remove_null(schema: &mut Schema) {
    if let Some(a @ Value::Array(_)) = schema.get_mut("type") {
        let arr = a.as_array_mut().unwrap();
        arr.retain(|v| matches!(v, Value::String(s) if s != "null"));
        if arr.len() == 1 {
            *a = arr.pop().unwrap();
        }
    }

    if let Some(a @ Value::Array(_)) = schema.get_mut("enum") {
        let arr = a.as_array_mut().unwrap();
        arr.retain(|v| matches!(v, Value::String(s) if s != "null"));
    }
}

pub trait Tool<State>: Serialize + DeserializeOwned {
    /// What this tool returns. `String` for plain text; see [`ToolOutput`].
    type Output: ToolOutput;

    /// Run the tool.
    ///
    /// `context` describes the caller for *this* request — the spec forbids
    /// carrying capabilities over from previous ones. On the command-line
    /// path it is [`RequestContext::default`], since there is no MCP client.
    fn execute(self, state: &mut State, context: &RequestContext) -> Result<Self::Output>;
}

/// Dispatch a decoded `tools/call` to the right tool.
///
/// Implemented by the [`tools!`](crate::tools) macro for the generated `Tools`
/// enum. Each tool has its own [`Tool::Output`], so dispatch erases them into
/// a single [`CallToolResult`].
pub trait Dispatch<State>: Sized + DeserializeOwned {
    /// Run the tool and build its result, keeping both the model-facing
    /// content and the program-facing structured value.
    fn call(self, state: &mut State, context: &RequestContext) -> Result<CallToolResult>;

    /// Run the tool and render it as plain text, for the command-line path.
    fn call_to_text(self, state: &mut State, context: &RequestContext) -> Result<String>;
}

/// Build a tool's advertised definition.
///
/// Generic over `State` because the output schema comes from
/// [`Tool::Output`], which is only nameable once `State` is known. The
/// `tools!` macro has it and supplies it.
pub trait AsToolSchema<State> {
    fn schema() -> ToolDefinition;
}

pub trait AsToolsList {
    fn tools_list() -> Vec<ToolDefinition>;
}

impl<T, State> AsToolSchema<State> for T
where
    T: JsonSchema + ToolMeta + Tool<State>,
{
    fn schema() -> ToolDefinition {
        let mut schema = schema_for::<Self>();

        let name = schema
            .remove("title")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let description = schema
            .remove("description")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();

        let examples = Self::examples();
        if !examples.is_empty() {
            schema.insert(
                "examples".to_string(),
                serde_json::to_value(examples).unwrap(),
            );
        }

        let mut tool = ToolDefinition::new(name, schema.into());
        tool.description = Some(description);
        tool.output_schema = <Self as Tool<State>>::Output::output_schema();
        tool.annotations = Self::annotations();
        tool.title = Self::title().map(String::from);
        tool.icons = Self::icons();
        tool
    }
}
