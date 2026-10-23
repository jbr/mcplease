use crate::State;
use anyhow::Result;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Wait until the test releases it, then report its place in the log.
///
/// Parks on a channel rather than blocking, so several of these can be
/// outstanding at once even when calls share one task — which is the point of
/// most of these tests. With `blocking` it blocks its thread instead, the way
/// a synchronous tool body does, which only a driver that gives each call its
/// own thread can run alongside anything else.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Block {
    #[serde(default)]
    pub blocking: bool,
}

impl ToolMeta for Block {}

impl Tool<State> for Block {
    type Output = String;

    async fn execute(self, state: State, context: RequestContext) -> Result<Self::Output> {
        let released = state.park();
        state.started.send(())?;
        if self.blocking {
            let _ = released.recv_blocking();
        } else {
            let _ = released.recv().await;
        }
        // The release is this tool's one checkpoint, so it is also where it
        // learns it is no longer wanted.
        context.progress(1.0, Some(1.0), Some("released"))?;
        Ok(format!("block #{}", state.record("block")))
    }
}
