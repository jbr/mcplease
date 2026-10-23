use crate::State;
use anyhow::Result;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Record a note and report its place in the log.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Note {
    pub text: String,
}

impl ToolMeta for Note {}

impl Tool<State> for Note {
    type Output = String;

    async fn execute(self, state: State, _context: RequestContext) -> Result<Self::Output> {
        Ok(format!("{} #{}", self.text, state.record(&self.text)))
    }
}
