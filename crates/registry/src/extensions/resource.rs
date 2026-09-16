//! Resource selection comes from host UI; extension readers cannot invoke writes.
use super::*;
use srelens_kube::gitops::ResourceIn;
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Selection {
    id: String,
    revision: u64,
    capability: String,
    context: String,
    namespace: String,
    name: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Action {
    resource: Selection,
    action: String,
    uid: String,
    #[serde(rename = "resourceVersion")]
    resource_version: String,
}
async fn resolve(
    path: PathBuf,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    selection: Selection,
) -> Result<ResourceIn, CapabilityError> {
    let resolved = request_context(&cache, &selection.context).await;
    let state = tokio::task::spawn_blocking(move || read(&path))
        .await
        .map_err(|e| CapabilityError::Handler(e.to_string()))?
        .map_err(CapabilityError::Handler)?;
    let plugin = state
        .plugins
        .iter()
        .find(|p| p.enabled && p.manifest.id == selection.id && p.revision == selection.revision)
        .ok_or_else(|| {
            CapabilityError::Handler(
                "App was disabled, removed or updated; refresh the view".into(),
            )
        })?;
    plugin.check_scope(&resolved)?;
    validate_app(&plugin.manifest, &plugin.grants, core)
        .map_err(|errors| CapabilityError::Handler(errors.to_string()))?;
    let binding = plugin
        .manifest
        .capabilities
        .iter()
        .find(|b| b.name == selection.capability && b.target == "k8s.listCustomResource")
        .ok_or_else(|| {
            CapabilityError::InvalidInput("A declared custom-resource reader is required".into())
        })?;
    let field = |key: &str| {
        binding.arguments[key]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    let resource = ResourceIn {
        // The pinned ID of the context scope was checked as (see `request_context`).
        context: resolved
            .ok()
            .and_then(|context| context.pinned_id())
            .unwrap_or(selection.context),
        namespace: selection.namespace,
        name: selection.name,
        group: field("group"),
        version: field("version"),
        plural: field("plural"),
        kind: field("kind"),
        namespaced: binding.arguments["namespaced"] == true,
    };
    resource.validate().map_err(CapabilityError::InvalidInput)?;
    Ok(resource)
}
pub(super) fn register(
    reg: &mut Registry,
    path: PathBuf,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
) {
    let p = path.clone();
    let c = core.clone();
    let k = cache.clone();
    reg.register(Capability::typed::<Selection, Value, _, _>(
        "extensions.resource",
        "Inspect the selected resource of an enabled app",
        Annotations::READ_ONLY,
        move |selection| {
            let p = p.clone();
            let c = c.clone();
            let k = k.clone();
            async move {
                let resource = resolve(p, c.clone(), k, selection).await?;
                c.invoke(
                    "k8s.getCustomResource",
                    serde_json::to_value(resource).unwrap(),
                )
                .await
            }
        },
    ));
    reg.register(Capability::typed::<Action, Value, _, _>("extensions.action", "Request a host-owned GitOps action on an app resource; requires explicit confirmation", Annotations::MUTATING, move |input| {
        let p = path.clone(); let c = core.clone(); let k = cache.clone(); async move {
            let resource = resolve(p,c.clone(),k,input.resource).await?;
            c.invoke("k8s.gitOpsAction", json!({"resource":resource,"action":input.action,"uid":input.uid,"resourceVersion":input.resource_version})).await
        }
    }));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn resource_identity_is_bound_to_inventory_and_lifecycle_is_rechecked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let core = super::super::tests::fake_core();
        let revision = super::super::tests::install(&path, core.clone());
        let payload = json!({"id":"org.example.argocd","revision":revision,"capability":"applications","context":"cluster/a","namespace":"team","name":"app"});
        // Use the actual binding name rather than inventing a second wire contract.
        let state = read(&path).unwrap();
        let mut payload = payload;
        payload["capability"] = json!(state.plugins[0].manifest.capabilities[0].name);
        let resolved = resolve(
            path.clone(),
            core.clone(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            serde_json::from_value(payload.clone()).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(resolved.context, "cluster/a");
        assert_eq!(resolved.namespace, "team");
        assert_eq!(resolved.group, "argoproj.io");
        let mut forged = payload.clone();
        forged["group"] = json!("other.io");
        assert!(serde_json::from_value::<Selection>(forged).is_err());
        mutate(
            &path,
            core.clone(),
            Configure::Enable {
                id: "org.example.argocd".into(),
                enabled: false,
            },
        )
        .unwrap();
        assert!(resolve(
            path,
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            serde_json::from_value(payload).unwrap()
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn a_selection_goes_to_the_cluster_its_scope_was_checked_against() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let core = super::super::tests::fake_core();
        let revision = super::super::tests::install(&path, core.clone());
        let config = super::super::tests::kubeconfig(dir.path(), "first.yaml", &["default"]);
        let binding = read(&path).unwrap().plugins[0].manifest.capabilities[0]
            .name
            .clone();
        let selection = json!({"id":"org.example.argocd","revision":revision,"capability":binding,"context":"default","namespace":"team","name":"app"});
        let resolved = resolve(
            path,
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![config.clone()]),
            serde_json::from_value(selection).unwrap(),
        )
        .await
        .unwrap();
        // Inspection and actions go out under the ID scope was checked as, so the capability
        // cannot resolve the name again to a cluster that took it since.
        assert_eq!(
            resolved.context,
            format!("srelens-context:{}#default", config.display())
        );
    }
    #[tokio::test]
    async fn action_dispatch_uses_bound_api_and_mcp_cannot_bypass_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let mut core = crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        );
        for id in ["k8s.getCustomResource", "k8s.gitOpsAction"] {
            let mut cap = core.get(id).unwrap().clone();
            cap.handler = Arc::new(|args| Box::pin(async move { Ok(args) }));
            core.register(cap);
        }
        let core = Arc::new(core);
        let revision = super::super::tests::install(&path, core.clone());
        let binding = read(&path).unwrap().plugins[0].manifest.capabilities[0]
            .name
            .clone();
        let mut reg = Registry::new();
        register(
            &mut reg,
            path.clone(),
            core.clone(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        let selected = json!({"id":"org.example.argocd","revision":revision,"capability":binding,"context":"cluster/a","namespace":"team","name":"app"});
        let payload = json!({"resource":selected,"action":"sync","uid":"u","resourceVersion":"2"});
        let result = reg
            .invoke("extensions.action", payload.clone())
            .await
            .unwrap();
        assert_eq!(result["resource"]["group"], "argoproj.io");
        assert_eq!(result["resource"]["name"], "app");
        assert_eq!(result["resourceVersion"], "2");
        let mcp = srelens_mcp::McpServer::new(Arc::new(reg));
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"extensions.action","arguments":payload}});
        let response =
            srelens_mcp::stdio::handle_request(&mcp, &request, srelens_mcp::Transport::Stdio)
                .await
                .unwrap();
        assert_eq!(response["result"]["isError"], true);
        mutate(
            &path,
            core.clone(),
            Configure::Remove {
                id: "org.example.argocd".into(),
            },
        )
        .unwrap();
        assert!(resolve(
            path,
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            serde_json::from_value(selected).unwrap()
        )
        .await
        .is_err());
    }
}
