//! Capability registry — the single source of truth for backend operations.

mod annotations;
mod error;

pub use annotations::Annotations;
pub use error::CapabilityError;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type Handler =
    Arc<dyn Fn(Value) -> BoxFuture<Result<Value, CapabilityError>> + Send + Sync>;

#[derive(Clone)]
pub struct Capability {
    pub id: String,
    pub summary: String,
    pub annotations: Annotations,
    pub input_schema: Value,
    pub output_schema: Value,
    pub handler: Handler,
}

impl Capability {
    /// Build a read-only capability from an async closure.
    pub fn read_only<F, Fut>(id: &str, summary: &str, f: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, CapabilityError>> + Send + 'static,
    {
        Self {
            id: id.to_string(),
            summary: summary.to_string(),
            annotations: Annotations::READ_ONLY,
            input_schema: Value::Null,
            output_schema: Value::Null,
            handler: Arc::new(move |v| Box::pin(f(v))),
        }
    }

    pub fn typed<I, O, F, Fut>(
        id: &str,
        summary: &str,
        annotations: Annotations,
        f: F,
    ) -> Self
    where
        I: serde::de::DeserializeOwned + schemars::JsonSchema,
        O: serde::Serialize + schemars::JsonSchema,
        F: Fn(I) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, CapabilityError>> + Send + 'static,
    {
        let input_schema = serde_json::to_value(schemars::schema_for!(I)).unwrap();
        let output_schema = serde_json::to_value(schemars::schema_for!(O)).unwrap();
        let handler: Handler = Arc::new(move |v: Value| {
            let parsed = serde_json::from_value::<I>(v);
            let fut = parsed.map(&f);
            Box::pin(async move {
                let input = fut.map_err(|e| CapabilityError::InvalidInput(e.to_string()))?;
                let out = input.await?;
                serde_json::to_value(out).map_err(|e| CapabilityError::Handler(e.to_string()))
            })
        });
        Self {
            id: id.to_string(),
            summary: summary.to_string(),
            annotations,
            input_schema,
            output_schema,
            handler,
        }
    }
}

#[derive(Default, Clone)]
pub struct Registry {
    caps: BTreeMap<String, Capability>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cap: Capability) {
        self.caps.insert(cap.id.clone(), cap);
    }

    pub fn ids(&self) -> Vec<&str> {
        self.caps.keys().map(String::as_str).collect()
    }

    pub fn entries(&self) -> impl Iterator<Item = &Capability> {
        self.caps.values()
    }

    pub fn get(&self, id: &str) -> Option<&Capability> {
        self.caps.get(id)
    }

    pub async fn invoke(&self, id: &str, input: Value) -> Result<Value, CapabilityError> {
        let cap = self
            .caps
            .get(id)
            .ok_or_else(|| CapabilityError::NotFound(id.to_string()))?;
        (cap.handler)(input).await
    }
}

/// Crate version sentinel used by the scaffold smoke test.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn invoke_returns_handler_output() {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("ping", "health check", |input| async move {
            Ok(json!({ "echo": input }))
        }));

        let out = reg.invoke("ping", json!("hi")).await.unwrap();
        assert_eq!(out, json!({ "echo": "hi" }));
    }

    #[tokio::test]
    async fn invoke_unknown_id_is_not_found() {
        let reg = Registry::new();
        let err = reg.invoke("nope", json!(null)).await.unwrap_err();
        assert!(matches!(err, CapabilityError::NotFound(_)));
    }

    #[test]
    fn ids_lists_registered_capabilities() {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("a", "", |_| async { Ok(json!(null)) }));
        reg.register(Capability::read_only("b", "", |_| async { Ok(json!(null)) }));
        let mut ids = reg.ids();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn get_returns_capability_for_registered_id() {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("a", "", |_| async { Ok(json!(null)) }));
        assert!(reg.get("a").is_some());
        assert!(reg.get("missing").is_none());
    }
}

#[cfg(test)]
mod typed_tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[derive(Deserialize, JsonSchema)]
    struct AddIn { a: i64, b: i64 }
    #[derive(Serialize, JsonSchema)]
    struct AddOut { sum: i64 }

    #[tokio::test]
    async fn typed_capability_roundtrips_and_has_schema() {
        let cap = Capability::typed::<AddIn, AddOut, _, _>(
            "math.add", "adds two ints", Annotations::READ_ONLY,
            |input| async move { Ok(AddOut { sum: input.a + input.b }) },
        );
        assert!(cap.input_schema.get("properties").is_some());

        let mut reg = Registry::new();
        reg.register(cap);
        let out = reg.invoke("math.add", json!({ "a": 2, "b": 3 })).await.unwrap();
        assert_eq!(out, json!({ "sum": 5 }));
    }

    #[tokio::test]
    async fn typed_capability_rejects_bad_input() {
        let cap = Capability::typed::<AddIn, AddOut, _, _>(
            "math.add", "", Annotations::READ_ONLY,
            |i| async move { Ok(AddOut { sum: i.a + i.b }) },
        );
        let mut reg = Registry::new();
        reg.register(cap);
        let err = reg.invoke("math.add", json!({ "a": "x" })).await.unwrap_err();
        assert!(matches!(err, CapabilityError::InvalidInput(_)));
    }
}
