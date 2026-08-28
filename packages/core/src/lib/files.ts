import { invokeCommand } from "../transport/transport";

/**
 * Prompt for a save location and write `content` there via the Rust backend
 * (the WebView's `<a download>` doesn't trigger a save in a Tauri webview).
 * Returns the chosen path, or null if the user cancelled.
 */
export async function saveTextFile(filename: string, content: string): Promise<string | null> {
  return (await invokeCommand<string | null>("save_text_file", { filename, content })) ?? null;
}

export async function pickKubeconfigFiles(): Promise<string[]> {
  return invokeCommand<string[]>("pick_kubeconfig_files");
}

export async function savePastedKubeconfig(content: string, name?: string): Promise<string> {
  return invokeCommand<string>("save_pasted_kubeconfig", { content, name });
}
