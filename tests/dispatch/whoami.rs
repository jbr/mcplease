use crate::State;
use anyhow::Result;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Report who the caller said it was.
///
/// Exists to make the `_meta` path observable from inside a tool: what a
/// client stamps onto a request is what this reads back out of its
/// [`RequestContext`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Whoami {}

impl ToolMeta for Whoami {}

impl Tool<State> for Whoami {
    type Output = String;

    async fn execute(self, _state: State, context: RequestContext) -> Result<Self::Output> {
        let caller = match context.client_info() {
            Some(client) => format!("{} {}", client.name, client.version),
            None => "anonymous".to_string(),
        };
        let version = context.protocol_version().unwrap_or("unstated");
        Ok(format!(
            "{caller} on {version}, elicitation: {}",
            context.supports_elicitation()
        ))
    }
}
