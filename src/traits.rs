use crate::types::{Example, ToolSchema};
use anyhow::Result;
use schemars::{
    JsonSchema, Schema,
    generate::SchemaSettings,
    transform::{RecursiveTransform, Transform},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub trait WithExamples: Sized + Serialize {
    fn examples() -> Vec<Example<Self>> {
        vec![]
    }
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
    fn execute(self, state: &mut State) -> Result<String>;
}

pub trait AsToolSchema {
    fn schema() -> ToolSchema;
}

pub trait AsToolsList {
    fn tools_list() -> Vec<ToolSchema>;
}

impl<T> AsToolSchema for T
where
    T: JsonSchema + WithExamples,
{
    fn schema() -> ToolSchema {
        let settings = SchemaSettings::draft2020_12().with(|s| {
            s.meta_schema = None;
            s.inline_subschemas = true;
        });

        let generator = settings.into_generator();
        let mut schema = generator.into_root_schema_for::<Self>();

        RecursiveTransform(remove_null).transform(&mut schema);

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
        schema.remove("$schema");

        let examples = Self::examples();
        if examples.is_empty() {
            schema.insert(
                "examples".to_string(),
                serde_json::to_value(examples).unwrap(),
            );
        }

        let value: Value = schema.into();
        let input_schema = match serde_json::from_value(value.clone()) {
            Ok(input_schema) => input_schema,
            Err(e) => {
                let json = serde_json::to_string_pretty(&value).unwrap();
                log::error!("could not parse input schema:\n{e}\n\n{json}");
                eprintln!("could not parse input schema:\n{e}\n\n{json}");
                panic!("{e}");
            }
        };

        ToolSchema {
            name,
            description: Some(description),
            input_schema,
        }
    }
}
