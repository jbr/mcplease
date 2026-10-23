use crate::State;
use anyhow::Result;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Report progress `count` times, then finish.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Tick {
    pub count: u32,
}

impl ToolMeta for Tick {}

impl Tool<State> for Tick {
    type Output = String;

    async fn execute(self, _state: State, context: RequestContext) -> Result<Self::Output> {
        for step in 1..=self.count {
            context.progress(f64::from(step), Some(f64::from(self.count)), Some("ticking"))?;
        }
        Ok(format!("ticked {}", self.count))
    }
}
