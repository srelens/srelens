// Typed wrappers for the two assistant-prompts commands (backend:
// `assistant_prompts.rs`, Task 21) — surfaces srelens's MCP diagnostic
// prompts (`srelens_mcp::prompts::PromptLibrary`) to the assistant's `/`
// slash menu. Field names are camelCase to mirror the Rust
// `PromptSummary`/`PromptArg` structs exactly (`#[serde(rename_all =
// "camelCase")]`) — no translation happens at this boundary.
import { invokeCommand } from "../transport/transport";

/** One declared prompt argument, as the backend reports it. */
export interface PromptArg {
  name: string;
  required: boolean;
  description: string | null;
}

/** One prompt as it's listed in the slash menu. */
export interface PromptSummary {
  name: string;
  description: string;
  arguments: PromptArg[];
}

/** List srelens's diagnostic prompts for the slash menu. */
export function listPrompts(): Promise<PromptSummary[]> {
  return invokeCommand("assistant_prompts_list");
}

/** Render one prompt by name, for the composer to drop into the input. */
export function getPrompt(name: string, args: Record<string, string>): Promise<string> {
  return invokeCommand("assistant_prompt_get", { name, args });
}
