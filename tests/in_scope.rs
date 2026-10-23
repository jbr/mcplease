//! `tools!(…, modules = false, …)`: two sets over the same tool types. The module
//! form cannot express this, since each set would declare a module per tool and a
//! module belongs to one parent.

#![cfg(feature = "server")]

use mcplease::{
    ServerConfig, handle_request,
    traits::AsToolsList,
    types::{Implementation, JsonRpcRequest, RequestContext},
};
use serde_json::json;

#[derive(Clone)]
pub struct State;

mod shared {
    use crate::State;
    use anyhow::Result;
    use mcplease::{
        traits::{Tool, ToolMeta},
        types::RequestContext,
    };
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// Answer "ping".
    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    pub struct Ping {}

    impl ToolMeta for Ping {}

    impl Tool<State> for Ping {
        type Output = String;

        async fn execute(self, _state: State, _context: RequestContext) -> Result<String> {
            Ok("ping".into())
        }
    }

    /// Answer "pong", at length.
    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    pub struct LongPong {}

    impl ToolMeta for LongPong {}

    impl Tool<State> for LongPong {
        type Output = String;

        async fn execute(self, _state: State, _context: RequestContext) -> Result<String> {
            Ok("pong".into())
        }
    }
}

mod narrow {
    use crate::{State, shared::Ping};

    mcplease::tools!(cli = false, modules = false, State, (Ping, "ping"));
}

mod wide {
    use crate::{
        State,
        shared::{LongPong as Pong, Ping},
    };

    mcplease::tools!(
        cli = false,
        modules = false,
        State,
        (Ping, "ping"),
        (Pong, "pong"),
    );
}

#[test]
fn each_set_lists_its_own_tools() {
    assert_eq!(narrow::Tools::tools_list().len(), 1);
    assert_eq!(wide::Tools::tools_list().len(), 2);
}

#[test]
fn a_renamed_type_dispatches_under_its_set_name() {
    let request: JsonRpcRequest = serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "pong", "arguments": {}},
    }))
    .unwrap();
    let context = RequestContext::from_request(&request);
    let config = ServerConfig::new(Implementation::new("test-server", "1.2.3"));
    let response = pollster::block_on(handle_request::<wide::Tools, State>(
        request, State, context, &config,
    ))
    .expect("nothing here is cancellable");
    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["result"]["content"][0]["text"], "pong");
}
