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
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct Whoami {}

impl ToolMeta for Whoami {}

impl Tool<State> for Whoami {
    type Output = String;

    fn execute(self, _state: &mut State, context: &RequestContext) -> Result<Self::Output> {
        let caller = match &context.client_info {
            Some(client) => format!("{} {}", client.name, client.version),
            None => "anonymous".to_string(),
        };
        let version = context.protocol_version.as_deref().unwrap_or("unstated");
        Ok(format!(
            "{caller} on {version}, elicitation: {}",
            context.supports_elicitation()
        ))
    }
}
