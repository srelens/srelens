import { invokeCommand } from "../transport/transport";

/** Providers the native srelens agent can drive. Serialized to match the Rust
 * `ProviderKind` (serde camelCase). */
export type ProviderKind = "anthropic" | "openAi" | "gemini" | "openAiCompatible";

export const PROVIDERS: { kind: ProviderKind; label: string; needsBaseUrl: boolean }[] = [
  { kind: "anthropic", label: "Anthropic", needsBaseUrl: false },
  { kind: "openAi", label: "OpenAI", needsBaseUrl: false },
  { kind: "gemini", label: "Google Gemini", needsBaseUrl: false },
  { kind: "openAiCompatible", label: "OpenAI-compatible", needsBaseUrl: true },
];

/** Non-secret native-agent settings. `models`/`baseUrls` are keyed by the
 * provider slug the backend uses (anthropic / openai / gemini / openai-compatible). */
export interface LlmSettings {
  defaultProvider: ProviderKind;
  models: Record<string, string>;
  baseUrls: Record<string, string>;
  maxTokens: number;
}

export interface ModelInfo {
  id: string;
  displayName: string;
}

/** Provider slug used as the key in `LlmSettings.models`/`baseUrls` — mirrors
 * `llm_config::slug` on the backend. */
export function providerSlug(kind: ProviderKind): string {
  switch (kind) {
    case "anthropic":
      return "anthropic";
    case "openAi":
      return "openai";
    case "gemini":
      return "gemini";
    case "openAiCompatible":
      return "openai-compatible";
  }
}

export function llmGetSettings(): Promise<LlmSettings> {
  return invokeCommand("llm_get_settings");
}

export function llmSetSettings(settings: LlmSettings): Promise<void> {
  return invokeCommand("llm_set_settings", { settings });
}

export function llmSetKey(provider: ProviderKind, key: string): Promise<void> {
  return invokeCommand("llm_set_key", { provider, key });
}

export function llmClearKey(provider: ProviderKind): Promise<void> {
  return invokeCommand("llm_clear_key", { provider });
}

/** The providers that currently have a key configured (never returns the key). */
export function llmKeyStatus(): Promise<ProviderKind[]> {
  return invokeCommand("llm_key_status");
}

/** Fetch a provider's models. `baseUrl` overrides the stored one so the
 * OpenAI-compatible setup flow can fetch against a just-typed URL before the
 * settings are saved. */
export function llmListModels(provider: ProviderKind, baseUrl?: string): Promise<ModelInfo[]> {
  return invokeCommand("llm_list_models", { provider, baseUrl });
}
