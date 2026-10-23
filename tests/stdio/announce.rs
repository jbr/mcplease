use crate::State;
use anyhow::Result;
use mcplease::{
    claude_channel::ChannelEvent,
    traits::{Tool, ToolMeta},
    types::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Send a channel event through the connection's sink, then finish.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Announce {
    pub content: String,
}

impl ToolMeta for Announce {}

impl Tool<State> for Announce {
    type Output = String;

    async fn execute(self, _state: State, context: RequestContext) -> Result<Self::Output> {
        let notifier = context
            .connection_notifier()
            .ok_or_else(|| anyhow::anyhow!("no connection notifier"))?;
        notifier.notify(ChannelEvent::new(self.content).with_meta("source_id", "7").into());
        Ok("announced".into())
    }
}
