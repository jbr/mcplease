use crate::State;
use anyhow::{Result, bail};
use mcplease::{
    traits::{Tool, ToolMeta},
    types::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Echo a message back, with a greeting.
///
/// The `clap::Args` derive is what lets `tools!` fold this into a `Subcommand`,
/// so it is needed exactly when mcplease's `cli` feature is on — a server on
/// another transport does not derive it, and does not depend on clap.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Echo {
    /// What to echo.
    pub message: String,
}

impl ToolMeta for Echo {}

impl Tool<State> for Echo {
    type Output = String;

    async fn execute(self, state: State, _context: RequestContext) -> Result<Self::Output> {
        if self.message.is_empty() {
            bail!("nothing to echo");
        }
        Ok(format!("{}, {}", state.greeting, self.message))
    }
}
