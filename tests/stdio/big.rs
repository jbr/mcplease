use crate::State;
use anyhow::Result;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Return a response of the requested size.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Big {
    pub bytes: usize,
}

impl ToolMeta for Big {}

impl Tool<State> for Big {
    type Output = String;

    async fn execute(self, _state: State, _context: RequestContext) -> Result<Self::Output> {
        Ok("x".repeat(self.bytes))
    }
}
