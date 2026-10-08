//! Native package uploads share validation, lifecycle and auditing with JSON
//! capabilities, without serializing the archive into base64 or JSON arrays.
use super::*;
use std::sync::Mutex;

pub(super) fn registry(
    registry: &Registry,
    id: &str,
    bytes: Vec<u8>,
    apps: Apps,
    core: Arc<Registry>,
    secrets: Arc<dyn srelens_plugin_host::SecretStore>,
) -> Result<Registry, CapabilityError> {
    if !matches!(id, "extensions.configure" | "extensions.packageManifest")
        || bytes.len() > package::MAX_PACKAGE_BYTES
    {
        return Err(CapabilityError::InvalidInput(
            "Raw uploads only review or install packages up to 512 MiB".into(),
        ));
    }
    let mut capability = registry
        .get(id)
        .cloned()
        .ok_or_else(|| CapabilityError::NotFound(id.into()))?;
    let review = id == "extensions.packageManifest";
    let bytes = Arc::new(Mutex::new(Some(bytes)));
    capability.handler = Arc::new(move |input| {
        let (apps, core, secrets, bytes) =
            (apps.clone(), core.clone(), secrets.clone(), bytes.clone());
        Box::pin(async move {
            let bytes = bytes.lock().unwrap().take().ok_or_else(|| {
                CapabilityError::InvalidInput(
                    "This package upload has already been consumed".into(),
                )
            })?;
            tokio::task::spawn_blocking(move || {
                if input.get("package").and_then(Value::as_str) != Some("") {
                    return Err(CapabilityError::InvalidInput(
                        "A raw upload requires an empty package marker".into(),
                    ));
                }
                if review {
                    let _: PackageIn = serde_json::from_value(input)
                        .map_err(|e| CapabilityError::InvalidInput(e.to_string()))?;
                    let verified = read_package(&bytes, &apps.catalog.authority())
                        .map_err(CapabilityError::Handler)?;
                    package::check_installable(&verified).map_err(CapabilityError::Handler)?;
                    serde_json::to_value(catalog::Review::of_package(&verified))
                        .map_err(|e| CapabilityError::Handler(e.to_string()))
                } else {
                    let mut command: Configure = serde_json::from_value(input)
                        .map_err(|e| CapabilityError::InvalidInput(e.to_string()))?;
                    let Configure::InstallPackage {
                        ref mut package, ..
                    } = command
                    else {
                        return Err(CapabilityError::InvalidInput(
                            "Raw uploads only install packages".into(),
                        ));
                    };
                    *package = bytes;
                    let inventory = configure(&apps, core, secrets.as_ref(), command)
                        .map_err(CapabilityError::Handler)?;
                    serde_json::to_value(inventory)
                        .map_err(|e| CapabilityError::Handler(e.to_string()))
                }
            })
            .await
            .map_err(|e| CapabilityError::Handler(e.to_string()))?
        })
    });
    let mut local = Registry::new();
    local.register(capability);
    Ok(local)
}
