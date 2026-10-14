//! Native executable operations use the same checked path as installed-app tools.
use super::{streams::ExtensionStreams, tools::AppTools};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};
use srelens_capability::{Annotations, Capability, Registry};
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CallOperation {
    pub id: String,
    pub revision: u64,
    pub context: String,
    pub operation: String,
    pub params: Map<String, Value>,
}

pub(super) fn register(reg: &mut Registry, streams: Arc<ExtensionStreams>) {
    reg.register(
        Capability::typed::<CallOperation, Value, _, _>(
            "extensions.callOperation",
            "Run a declared executable app operation on its pinned cluster",
            Annotations {
                sensitive: true,
                ..Annotations::READ_ONLY
            },
            move |input| {
                let tools: Arc<AppTools> = streams.app_tools();
                async move { tools.call_native(input).await }
            },
        )
        .only_in_the_ui(),
    );
}
