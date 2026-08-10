//! Surfaces srelens's MCP diagnostic prompts (`srelens_mcp::prompts`) to the
//! in-app AI assistant's `/` slash menu — the same `PromptLibrary` an MCP
//! client would see via `prompts/list`/`prompts/get`, read in-process here
//! (no HTTP, no token) so the assistant composer can list and render them.
//!
//! As with `assistant_history`, the `#[tauri::command]` wrappers at the
//! bottom only resolve the managed prompts dir and delegate; every real
//! decision lives in the pure `fn`s above them, which take a `&PromptLibrary`
//! so tests can drive them against builtins only (`PromptLibrary::new(None)`)
//! with no filesystem involved.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use srelens_mcp::prompts::PromptLibrary;

/// One declared prompt argument, as the client sees it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptArg {
    pub name: String,
    pub required: bool,
    pub description: Option<String>,
}

/// One prompt as it's listed in the slash menu: enough to render a name +
/// description entry and know which arguments a picked prompt takes.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptSummary {
    pub name: String,
    pub description: String,
    pub arguments: Vec<PromptArg>,
}

/// Map every `PromptSpec` in `lib` to a `PromptSummary` for the slash menu.
fn summaries(lib: &PromptLibrary) -> Vec<PromptSummary> {
    lib.list()
        .into_iter()
        .map(|spec| PromptSummary {
            name: spec.name,
            description: spec.description,
            arguments: spec
                .arguments
                .into_iter()
                .map(|a| PromptArg {
                    name: a.name,
                    required: a.required,
                    description: a.description,
                })
                .collect(),
        })
        .collect()
}

/// Render `name` for `args`, returning just the text the composer drops into
/// the input — the `description` half of `Rendered` is only useful to an MCP
/// client deciding whether to show the prompt at all, which the slash menu
/// already did via `summaries`. Unknown name (or any other rejection from
/// `PromptLibrary::get`, e.g. a required arg that's missing) surfaces as-is.
fn render_prompt(lib: &PromptLibrary, name: &str, args: HashMap<String, String>) -> Result<String, String> {
    let supplied: BTreeMap<String, String> = args.into_iter().collect();
    lib.get(name, &supplied).map(|rendered| rendered.text)
}

/// List srelens's diagnostic prompts (built-ins plus any user-authored files
/// under `<config>/mcp/prompts`) for the assistant's slash menu.
#[tauri::command]
pub fn assistant_prompts_list(
    prompts_dir: tauri::State<'_, crate::mcp::McpPromptsDir>,
) -> Vec<PromptSummary> {
    summaries(&PromptLibrary::new(Some(prompts_dir.0.clone())))
}

/// Render one prompt by name for the composer to drop into the input,
/// unedited, for the user to review/edit/send.
#[tauri::command]
pub fn assistant_prompt_get(
    name: String,
    args: HashMap<String, String>,
    prompts_dir: tauri::State<'_, crate::mcp::McpPromptsDir>,
) -> Result<String, String> {
    render_prompt(&PromptLibrary::new(Some(prompts_dir.0.clone())), &name, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_includes_a_builtin_with_a_non_empty_description_and_mapped_arguments() {
        let lib = PromptLibrary::new(None);
        let list = summaries(&lib);

        let pod_crashloop = list
            .iter()
            .find(|p| p.name == "pod-crashloop")
            .expect("builtin `pod-crashloop` should be listed");
        assert!(!pod_crashloop.description.is_empty());
        assert!(
            !pod_crashloop.arguments.is_empty(),
            "pod-crashloop declares arguments (context, namespace, pod) that should carry through"
        );
        let context_arg = pod_crashloop
            .arguments
            .iter()
            .find(|a| a.name == "context")
            .expect("pod-crashloop declares a `context` argument");
        assert!(context_arg.required, "`context` is required on every variant of pod-crashloop");
    }

    #[test]
    fn render_prompt_on_a_builtin_with_its_required_args_returns_non_empty_text() {
        let lib = PromptLibrary::new(None);
        let mut args = HashMap::new();
        args.insert("context".to_string(), "my-cluster".to_string());

        let text = render_prompt(&lib, "pod-crashloop", args).expect("pod-crashloop should render with `context` supplied");
        assert!(!text.is_empty());
    }

    #[test]
    fn render_prompt_on_an_unknown_name_returns_err_distinct_from_a_real_render() {
        let lib = PromptLibrary::new(None);
        let mut args = HashMap::new();
        args.insert("context".to_string(), "my-cluster".to_string());
        let ok_text = render_prompt(&lib, "pod-crashloop", args).unwrap();

        let err = render_prompt(&lib, "not-a-real-prompt", HashMap::new())
            .expect_err("an unknown prompt name should be rejected");
        assert!(!err.is_empty());
        assert_ne!(err, ok_text, "the error path must not coincide with a real rendered prompt");
    }
}
