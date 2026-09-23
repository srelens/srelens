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
) -> Result<(ResourceIn, Installed), CapabilityError> {
    let resolved = request_context(&cache, &selection.context).await;
    let state = tokio::task::spawn_blocking(move || read(&path))
        .await
        .map_err(|e| CapabilityError::Handler(e.to_string()))?
        .map_err(CapabilityError::Handler)?;
    if let Some(reason) = state
        .plugins
        .iter()
        .find(|p| p.manifest.id == selection.id)
        .and_then(|p| p.policy_blocked.as_ref())
    {
        return Err(CapabilityError::Handler(reason.clone()));
    }
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
    validate_app(&plugin.manifest, &plugin.grants, core.clone())
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
    // Before `k8s.getCustomResource` or a declared action sees it: a whole built-in object,
    // such as a Deployment with its environment, must not come back through an app (#601).
    crd::require(&core, &resource.context, binding).await?;
    Ok((resource, plugin.clone()))
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
                let binding_name = selection.capability.clone();
                let (resource, plugin) = resolve(p, c.clone(), k, selection).await?;
                let mut detail = c.invoke("k8s.getCustomResource", serde_json::to_value(resource).unwrap()).await?;
                let mut meta = serde_json::Map::new();
                let mut actions = Vec::new();
                for action in plugin.manifest.actions.iter().filter(|a| a.resource == binding_name) {
                    let annotations = c.get(&action.target).ok_or_else(|| CapabilityError::Handler("Action primitive unavailable".into()))?.annotations;
                    actions.push(action.name.clone());
                    meta.insert(action.name.clone(), json!({"title":action.title,"availableWhen":action.available_when,"impact":annotations.impact,"confirm":annotations.confirm}));
                }
                detail["actions"] = json!(actions);
                detail["actionMeta"] = Value::Object(meta);
                Ok(detail)
            }
        },
    ));
    // The generic endpoint carries the highest primitive impact. Per-action
    // wording and impact come exclusively from the host primitive metadata.
    reg.register(Capability::typed::<Action, Value, _, _>("extensions.action", "Run a declared action on an app resource; requires explicit confirmation", Annotations::MUTATING.with_impact(srelens_capability::Impact::High).with_confirm("Run the declared action[ ({action})][ on {resource}][ in cluster {cluster}]?"), move |input| {
        let p = path.clone(); let c = core.clone(); let k = cache.clone(); async move {
            let binding_name = input.resource.capability.clone();
            let (resource, plugin) = resolve(p,c.clone(),k,input.resource).await?;
            if !plugin.manifest.actions.iter().any(|a| a.name == input.action && a.resource == binding_name) {
                return Err(CapabilityError::InvalidInput("This action is not declared for the selected resource".into()));
            }
            let id = format!("plugin/{}/{}", plugin.manifest.id, input.action);
            let mut registry = Registry::new();
            let _registration = PluginHost::new(c).register(&mut registry, plugin.manifest, &plugin.grants).map_err(CapabilityError::Handler)?;
            registry.invoke(&id, json!({"context":resource.context,"namespace":resource.namespace,"name":resource.name,"uid":input.uid,"resourceVersion":input.resource_version})).await
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
        assert_eq!(resolved.0.context, "cluster/a");
        assert_eq!(resolved.0.namespace, "team");
        assert_eq!(resolved.0.group, "argoproj.io");
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
            resolved.0.context,
            format!("srelens-context:{}#default", config.display())
        );
    }
    /// Neither `k8s.getCustomResource` nor a declared action is reached for a binding that
    /// is not a CustomResourceDefinition on the cluster, stored or installed (#601).
    #[tokio::test]
    async fn resources_and_actions_are_refused_without_a_matching_crd() {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let mut core = (*super::super::tests::fake_core()).clone();
        for id in ["k8s.getCustomResource", "k8s.mergePatch"] {
            let mut cap = core.get(id).unwrap().clone();
            cap.handler = Arc::new(|args| {
                CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async move { Ok(args) })
            });
            core.register(cap);
        }
        // The cluster serves Argo CD's CRD, but not one for the aggregated group below.
        super::super::tests::serve_crds(&mut core, &["applications.argoproj.io/v1alpha1"]);
        let core = Arc::new(core);
        let mut source: Value = serde_json::from_str(&super::super::tests::manifest()).unwrap();
        source["capabilities"][0]["arguments"]["group"] = json!("apps.openshift.io");
        mutate(
            &path,
            core.clone(),
            Configure::Install {
                signature: None,
                manifest: source.to_string(),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: None,
            },
        )
        .unwrap();
        let state = read(&path).unwrap();
        let binding = state.plugins[0].manifest.capabilities[0].name.clone();
        let mut reg = Registry::new();
        register(
            &mut reg,
            path.clone(),
            core.clone(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        let selected = |revision: u64| json!({"id":"org.example.argocd","revision":revision,"capability":binding,"context":"cluster/a","namespace":"team","name":"app"});
        let revision = state.plugins[0].revision;
        let refused = reg
            .invoke("extensions.resource", selected(revision))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("No CustomResourceDefinition applications.apps.openshift.io"),
            "{refused}"
        );
        let action =
            json!({"resource":selected(revision),"action":"sync","uid":"u","resourceVersion":"2"});
        assert!(reg.invoke("extensions.action", action).await.is_err());

        // A stored binding of apps/v1 deployments cannot return a Deployment either.
        let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let arguments = &mut stored["plugins"][0]["manifest"]["capabilities"][0]["arguments"];
        arguments["group"] = json!("apps");
        arguments["plural"] = json!("deployments");
        arguments["kind"] = json!("Deployment");
        stored["plugins"][0]["enabled"] = json!(true);
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(reg
            .invoke("extensions.resource", selected(revision))
            .await
            .is_err());
        assert_eq!(CALLS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn action_dispatch_uses_bound_api_and_mcp_cannot_bypass_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let mut core = crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        );
        for id in ["k8s.getCustomResource", "k8s.mergePatch"] {
            let mut cap = core.get(id).unwrap().clone();
            cap.handler = Arc::new(|args| Box::pin(async move { Ok(args) }));
            core.register(cap);
        }
        super::super::tests::serve_crds(&mut core, &["applications.argoproj.io/v1alpha1"]);
        let core = Arc::new(core);
        let mut manifest: Value =
            serde_json::from_str(include_str!("../../../../examples/extensions/argocd.json"))
                .unwrap();
        manifest["id"] = json!("org.example.argocd");
        mutate(
            &path,
            core.clone(),
            Configure::UnsignedApps {
                allow_unsigned_apps: true,
            },
        )
        .unwrap();
        mutate(
            &path,
            core.clone(),
            Configure::Install {
                signature: None,
                manifest: manifest.to_string(),
                grants: vec![
                    "k8s.listCustomResource".into(),
                    "k8s.annotate".into(),
                    "k8s.mergePatch".into(),
                ],
                reviewed_revision: None,
            },
        )
        .unwrap();
        let revision = read(&path).unwrap().plugins[0].revision;
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
        let detail = reg
            .invoke("extensions.resource", selected.clone())
            .await
            .unwrap();
        assert_eq!(
            detail["actions"],
            json!(["refresh", "hard-refresh", "sync"])
        );
        assert_eq!(detail["actionMeta"]["sync"]["title"], "Sync");
        assert_eq!(detail["actionMeta"]["sync"]["impact"], "high");
        assert_eq!(
            detail["actionMeta"]["sync"]["availableWhen"][0]["jsonPath"],
            ".operation"
        );
        let forged =
            json!({"resource":selected,"action":"sync","uid":"u","resourceVersion":"2","patch":{}});
        assert!(reg.invoke("extensions.action", forged).await.is_err());
        let payload = json!({"resource":selected,"action":"sync","uid":"u","resourceVersion":"2"});
        let result = reg
            .invoke("extensions.action", payload.clone())
            .await
            .unwrap();
        assert_eq!(result["group"], "argoproj.io");
        assert_eq!(result["name"], "app");
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

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn gitops_examples_declare_writes_and_core_has_no_implicit_action() {
        let core = super::super::tests::fake_core();
        assert!(
            core.get("k8s.gitOpsAction").is_none(),
            "GitOps writes must only use declared primitives"
        );
        for source in [
            include_str!("../../../../examples/extensions/flux.json"),
            include_str!("../../../../examples/extensions/argocd.json"),
        ] {
            let manifest = Manifest::parse(source).unwrap();
            assert!(
                !manifest.actions.is_empty(),
                "{} must declare its actions",
                manifest.id
            );
            validate_app(&manifest, &manifest.permissions, core.clone()).unwrap();
            assert!(
                validate_app(&manifest, &["k8s.listCustomResource".into()], core.clone()).is_err()
            );
        }
    }
    #[tokio::test]
    async fn reader_only_app_cannot_inherit_gitops_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let mut core = (*super::super::tests::fake_core()).clone();
        if let Some(mut cap) = core.get("k8s.gitOpsAction").cloned() {
            cap.handler = Arc::new(|_| Box::pin(async { Ok(json!({"requested":true})) }));
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
            path,
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        let result = reg.invoke("extensions.action", json!({"resource":{"id":"org.example.argocd","revision":revision,"capability":binding,"context":"cluster/a","namespace":"team","name":"app"},"action":"sync","uid":"u","resourceVersion":"2"})).await;
        assert!(result.is_err(), "A reader grant must never grant a write");
    }
}

#[cfg(test)]
mod declaration_tests {
    use super::*;
    #[test]
    fn flux_and_argo_actions_preserve_existing_patches_and_preconditions() {
        let flux =
            Manifest::parse(include_str!("../../../../examples/extensions/flux.json")).unwrap();
        assert_eq!(flux.actions.len(), 29);
        for reader in &flux.capabilities {
            let actions: Vec<_> = flux
                .actions
                .iter()
                .filter(|a| a.resource == reader.name)
                .collect();
            let kind = reader
                .arguments
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("");
            let count = match kind {
                "HelmRelease" => 5,
                "Kustomization"
                | "GitRepository"
                | "HelmRepository"
                | "HelmChart"
                | "Bucket"
                | "OCIRepository"
                | "ImageRepository"
                | "ImageUpdateAutomation" => 3,
                _ => 0,
            };
            assert_eq!(actions.len(), count, "{kind}");
            for action in actions {
                let verb = action
                    .name
                    .strip_prefix(&format!("{}-", reader.name))
                    .unwrap();
                let suspended = json!({"spec":{"suspend":true}});
                let active = json!({"spec":{"suspend":false}});
                let absent = json!({});
                let allowed = |resource: &Value| {
                    srelens_capability::unmet(&action.preconditions, resource).is_none()
                };
                assert_eq!(allowed(&suspended), verb == "resume", "{}", action.name);
                assert_eq!(allowed(&active), verb != "resume", "{}", action.name);
                assert_eq!(allowed(&absent), verb != "resume", "{}", action.name);
                assert_eq!(
                    serde_json::to_value(&action.preconditions).unwrap(),
                    serde_json::to_value(&action.available_when).unwrap()
                );
                let binding = flux.action_binding(action).unwrap();
                for field in ["group", "version", "plural", "kind", "namespaced"] {
                    assert_eq!(binding.arguments[field], reader.arguments[field]);
                }
                match verb {
                    "suspend" | "resume" => assert_eq!(
                        action.arguments["fields"],
                        json!({"/spec/suspend":verb == "suspend"})
                    ),
                    "reconcile" => assert_eq!(
                        action.arguments,
                        serde_json::from_value(
                            json!({"key":"reconcile.fluxcd.io/requestedAt","value":"$now"})
                        )
                        .unwrap()
                    ),
                    "force" | "reset" => assert_eq!(
                        action.arguments["patch"],
                        json!({"metadata":{"annotations":{"reconcile.fluxcd.io/requestedAt":"$now",format!("reconcile.fluxcd.io/{verb}At"):"$now"}}})
                    ),
                    _ => panic!("Unexpected action {verb}"),
                }
            }
        }
        let argo =
            Manifest::parse(include_str!("../../../../examples/extensions/argocd.json")).unwrap();
        assert_eq!(argo.actions.len(), 3);
        let sync = argo.actions.iter().find(|a| a.name == "sync").unwrap();
        assert_eq!(
            sync.arguments["patch"],
            json!({"operation":{"initiatedBy":{"username":"srelens"},"sync":{"prune":false,"syncStrategy":{"hook":{}}}}})
        );
        assert!(srelens_capability::unmet(&sync.preconditions, &json!({"operation":{}})).is_some());
        assert!(
            srelens_capability::unmet(&sync.preconditions, &json!({"operation":null})).is_none()
        );
        assert!(srelens_capability::unmet(&sync.preconditions, &json!({})).is_none());
        for action in &argo.actions[..2] {
            assert!(action.preconditions.is_empty());
            assert_eq!(action.arguments["key"], "argocd.argoproj.io/refresh");
            assert_eq!(
                action.arguments["value"],
                if action.name == "refresh" {
                    "normal"
                } else {
                    "hard"
                }
            );
        }
    }
}
