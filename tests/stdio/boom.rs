use crate::State;
use anyhow::Result;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Panic.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Boom {}

impl ToolMeta for Boom {}

impl Tool<State> for Boom {
    type Output = String;

    async fn execute(self, _state: State, _context: RequestContext) -> Result<Self::Output> {
        panic!("boom");
    }
}
