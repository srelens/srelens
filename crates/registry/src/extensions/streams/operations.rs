//! Declared sidecar streams retain native view and inventory ownership.
use super::{ExtensionStreams, OpenStreamIn, OpenStreamOut, StreamSourceIn};
use crate::extensions::operations::CallOperation;
use srelens_plugin_host::sidecar::StreamEvent;
use srelens_streams::{
    app::{StreamOwner, StreamWindow},
    EventSink,
};
use std::sync::Arc;

impl ExtensionStreams {
    pub(super) async fn open_operation(
        &self,
        sink: Arc<dyn EventSink>,
        window: Option<StreamWindow>,
        input: OpenStreamIn,
    ) -> Result<OpenStreamOut, String> {
        let StreamSourceIn::Operation { method, params } = &input.source else {
            unreachable!()
        };
        if !input.namespace.is_empty()
            && !crate::extensions::cards::namespace_name(&input.namespace)
        {
            return Err("Invalid operation namespace".into());
        }
        if params
            .get("namespace")
            .is_some_and(|value| value.as_str() != Some(input.namespace.as_str()))
        {
            return Err("The operation namespace differs from its view".into());
        }
        let tools = self.app_tools();
        let (app, params) = tools
            .native_authority(&CallOperation {
                id: input.id.clone(),
                revision: input.revision,
                context: input.context,
                operation: method.clone(),
                params: params.clone(),
            })
            .await
            .map_err(|error| error.to_string())?;
        if !app
            .manifest
            .operation(method)
            .and_then(|operation| operation.view.as_ref())
            .is_some_and(|view| view.stream)
        {
            return Err("This operation is not a declared streaming view".into());
        }
        let owner = StreamOwner {
            app: input.id,
            revision: input.revision,
            view: input.view,
            window,
        };
        let method = method.clone();
        let stream = self
            .streams
            .open(
                owner,
                "operation",
                sink,
                input.channel.clone(),
                move |tx| async move {
                    // Dropping this handle on native cancellation also cancels the
                    // sidecar stream, including its broker acquisitions.
                    let mut source = tools
                        .open_native_stream(&app, &method, params)
                        .await
                        .map_err(|error| error.to_string())?;
                    while let Some(event) = source.next().await {
                        match event {
                            StreamEvent::Data(value) => {
                                if tx.data(value).is_err() {
                                    return Ok(());
                                }
                            }
                            StreamEvent::Closed => return Ok(()),
                            StreamEvent::Failed(why) => return Err(why),
                        }
                    }
                    Err("The sidecar stream ended without a terminal response".into())
                },
            )
            .map_err(|error| error.to_string())?;
        Ok(OpenStreamOut {
            stream,
            channel: input.channel,
        })
    }
}
