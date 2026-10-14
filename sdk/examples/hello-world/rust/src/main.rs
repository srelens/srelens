//! The smallest srelens sidecar in Rust: an operation, a call to srelens on
//! the cluster the caller names, and a stream. Declare these operations in
//! the app's manifest (`docs/extensions/manifest.md`, "Executable apps").

use serde::{Deserialize, Serialize};
use serde_json::Value;
use srelens_sidecar::{CallContext, Context, Error, Frames, Sidecar};

#[derive(Deserialize)]
struct GreetInput {
    name: String,
}

#[derive(Serialize)]
struct Greeting {
    greeting: String,
}

/// `greet {name}`.
async fn greet(_ctx: Context, input: GreetInput) -> Result<Greeting, Error> {
    log::info!("greeting {}", input.name);
    Ok(Greeting {
        greeting: format!("Hello, {}", input.name),
    })
}

#[derive(Deserialize)]
struct PodsInput {
    /// srelens adds no cluster to an operation's input: the operation
    /// declares one, and names it in every call it makes.
    cluster: String,
    #[serde(default)]
    namespace: Option<String>,
}

/// `pods {cluster, namespace?}`: the app's `pods` reader, through srelens.
async fn pods(ctx: Context, input: PodsInput) -> Result<Value, Error> {
    let context = CallContext::new(input.cluster, input.namespace.as_deref())?;
    Ok(ctx.host().read(&context, "pods").await?)
}

#[derive(Deserialize)]
struct CountInput {
    to: u64,
}

/// The stream `count {to}`: `0..to`, stopping when srelens cancels it.
async fn count(ctx: Context, input: CountInput, frames: Frames) -> Result<(), Error> {
    for n in 0..input.to {
        if ctx.is_cancelled() {
            break;
        }
        frames.send(&n).await?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    Sidecar::new("hello-world", env!("CARGO_PKG_VERSION"))
        .operation("greet", greet)
        .operation("pods", pods)
        .stream("count", count)
        .run_stdio()
        .await
}
