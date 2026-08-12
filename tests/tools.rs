//! The tool-authoring surface: what `Tool`, `ToolOutput`, and `ToolMeta`
//! produce on the wire.

use anyhow::Result;
use mcplease::{
    structured_output,
    traits::{AsToolSchema, Tool, ToolMeta, ToolOutput},
    types::{CallToolResult, ContentBlock, Example, RequestContext, ToolAnnotations},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Look up the weather.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct GetWeather {
    /// City name or zip code.
    location: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Weather {
    temperature: f64,
    conditions: String,
}

structured_output!(Weather);

impl ToolMeta for GetWeather {
    fn examples() -> Vec<Example<Self>> {
        vec![Example {
            description: "by city",
            item: Self {
                location: "Portland".into(),
            },
        }]
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations {
            read_only_hint: Some(true),
            open_world_hint: Some(true),
            ..ToolAnnotations::default()
        })
    }

    fn title() -> Option<&'static str> {
        Some("Weather Lookup")
    }
}

impl Tool<()> for GetWeather {
    type Output = Weather;

    fn execute(self, _state: &mut (), _context: &RequestContext) -> Result<Self::Output> {
        Ok(Weather {
            temperature: 22.5,
            conditions: "Partly cloudy".into(),
        })
    }
}

/// Echo something back.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Echo {
    message: String,
}

impl ToolMeta for Echo {}

impl Tool<()> for Echo {
    type Output = String;

    fn execute(self, _state: &mut (), _context: &RequestContext) -> Result<Self::Output> {
        Ok(self.message)
    }
}

#[test]
fn structured_output_advertises_a_schema_and_fills_both_halves() {
    let schema = <GetWeather as AsToolSchema<()>>::schema();
    let output_schema = schema.output_schema.expect("output schema advertised");
    assert_eq!(output_schema["properties"]["temperature"]["type"], "number");
    assert_eq!(output_schema["type"], "object");

    let weather = GetWeather {
        location: "Portland".into(),
    }
    .execute(&mut (), &RequestContext::default())
    .unwrap();

    // The spec's recommended shape: the structured value, plus its serialized
    // JSON as text so a client that ignores structuredContent still works.
    let result = CallToolResult::from_content(weather.to_content(), weather.structured_content());
    assert_eq!(
        result.structured_content,
        Some(json!({"temperature": 22.5, "conditions": "Partly cloudy"}))
    );
    let [ContentBlock::Text { text, .. }] = &result.content[..] else {
        panic!("expected one text block");
    };
    assert_eq!(
        serde_json::from_str::<Weather>(text).unwrap(),
        Weather {
            temperature: 22.5,
            conditions: "Partly cloudy".into()
        }
    );
}

#[test]
fn string_output_stays_plain_text_with_no_structured_half() {
    let schema = <Echo as AsToolSchema<()>>::schema();
    assert_eq!(schema.output_schema, None);
    assert_eq!(schema.title, None);
    assert!(schema.annotations.is_none());

    let output = Echo {
        message: "hello".into(),
    }
    .execute(&mut (), &RequestContext::default())
    .unwrap();

    // Not JSON-quoted: a String output is the text, verbatim.
    let [ContentBlock::Text { text, .. }] = &output.to_content()[..] else {
        panic!("expected one text block");
    };
    assert_eq!(text, "hello");
    assert_eq!(output.structured_content(), None);
    assert_eq!(output.to_text(), "hello");
}

#[test]
fn tool_meta_reaches_the_advertised_definition() {
    let schema = <GetWeather as AsToolSchema<()>>::schema();

    assert_eq!(schema.name, "GetWeather");
    assert_eq!(schema.description.as_deref(), Some("Look up the weather."));
    assert_eq!(schema.title.as_deref(), Some("Weather Lookup"));

    let annotations = schema.annotations.expect("annotations advertised");
    assert_eq!(annotations.read_only_hint, Some(true));
    // Undeclared hints stay absent rather than being guessed at; the spec
    // gives them pessimistic defaults on the client side.
    assert_eq!(annotations.destructive_hint, None);

    let examples = &schema.input_schema["examples"];
    assert_eq!(examples[0]["description"], "by city");
    assert_eq!(examples[0]["location"], "Portland");
}
