import { useEffect, useState } from "react";
import { Check, ChevronDown, RefreshCw, Trash2 } from "lucide-react";
import { Button, TextInput } from "../ui";
import { notify } from "../lib/notify";
import { listAgents, type AgentInfo } from "../lib/chat";
import {
  PROVIDERS,
  providerSlug,
  llmGetSettings,
  llmSetSettings,
  llmSetKey,
  llmClearKey,
  llmKeyStatus,
  llmListModels,
  type LlmSettings,
  type ModelInfo,
  type ProviderKind,
} from "../lib/llm";

const DEFAULT_SETTINGS: LlmSettings = {
  defaultProvider: "anthropic",
  models: {},
  baseUrls: {},
  maxTokens: 4096,
};

/**
 * Settings → Assistant. Configures srelens's own native agent: an API key per
 * provider (stored in the OS keychain), the default provider used in chat, and
 * the model per provider (fetched live from the provider's models API). The key
 * itself is never read back — the backend only reports which providers have one.
 */
export function AssistantSettingsSection() {
  const [settings, setSettings] = useState<LlmSettings>(DEFAULT_SETTINGS);
  const [keyed, setKeyed] = useState<ProviderKind[]>([]);
  const [keyDrafts, setKeyDrafts] = useState<Record<string, string>>({});
  const [models, setModels] = useState<Record<string, ModelInfo[]>>({});
  const [clis, setClis] = useState<AgentInfo[]>([]);
  const [busy, setBusy] = useState<string>("");
  // Which provider row is open for editing — one at a time; the rest collapse
  // to their one-line status. Starts on the default provider, since that's
  // the one the agent actually uses.
  const [expanded, setExpanded] = useState<ProviderKind | null>(null);

  useEffect(() => {
    llmGetSettings()
      .then((s) => {
        setSettings({ ...DEFAULT_SETTINGS, ...s });
        setExpanded(s.defaultProvider ?? DEFAULT_SETTINGS.defaultProvider);
      })
      .catch(() => {
        setSettings(DEFAULT_SETTINGS);
        setExpanded(DEFAULT_SETTINGS.defaultProvider);
      });
    refreshKeyStatus();
    // The vendor CLIs are detected on PATH; show their install status here too,
    // so this one screen covers every agent. Only installed CLIs are selectable
    // in the composer (the picker disables the rest).
    listAgents()
      .then((list) => setClis(list.filter((a) => a.kind !== "srelens")))
      .catch(() => setClis([]));
  }, []);

  function refreshKeyStatus() {
    llmKeyStatus()
      .then(setKeyed)
      .catch(() => setKeyed([]));
  }

  async function saveKey(provider: ProviderKind) {
    const draft = keyDrafts[provider]?.trim();
    if (!draft) return;
    try {
      await llmSetKey(provider, draft);
      setKeyDrafts((d) => ({ ...d, [provider]: "" }));
      refreshKeyStatus();
      notify.success("API key saved");
    } catch (e) {
      notify.error(String(e));
    }
  }

  async function clearKey(provider: ProviderKind) {
    try {
      await llmClearKey(provider);
      refreshKeyStatus();
      notify.success("API key removed");
    } catch (e) {
      notify.error(String(e));
    }
  }

  async function fetchModels(provider: ProviderKind) {
    setBusy(`models:${provider}`);
    try {
      // Pass the currently-typed base URL so first-time OpenAI-compatible setup
      // fetches against it without needing a separate "Save settings" first.
      const list = await llmListModels(provider, settings.baseUrls[providerSlug(provider)]);
      setModels((m) => ({ ...m, [provider]: list }));
      if (list.length === 0) notify.info("The provider returned no models");
    } catch (e) {
      notify.error(String(e));
    } finally {
      setBusy("");
    }
  }

  function setModel(provider: ProviderKind, model: string) {
    setSettings((s) => ({ ...s, models: { ...s.models, [providerSlug(provider)]: model } }));
  }

  function setBaseUrl(provider: ProviderKind, url: string) {
    setSettings((s) => ({ ...s, baseUrls: { ...s.baseUrls, [providerSlug(provider)]: url } }));
  }

  async function saveSettings() {
    setBusy("settings");
    try {
      await llmSetSettings(settings);
      notify.success("Assistant settings saved");
    } catch (e) {
      notify.error(String(e));
    } finally {
      setBusy("");
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold">srelens agent</h2>
        <p className="text-sm text-muted-foreground">
          srelens&apos;s own agent talks directly to a provider with your API key — no CLI to install.
          It can only operate the cluster through the srelens tools, and destructive actions still
          prompt for confirmation.
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <p className="text-xs text-muted-foreground">
          The selected provider is the one the srelens agent uses in chat. Click a row to configure
          it.
        </p>
        <div className="divide-y divide-border rounded-lg border border-border">
          {PROVIDERS.map((p) => {
            const slug = providerSlug(p.kind);
            const hasKey = keyed.includes(p.kind);
            const list = models[p.kind] ?? [];
            const model = settings.models[slug] ?? "";
            const isDefault = settings.defaultProvider === p.kind;
            const isOpen = expanded === p.kind;
            // The one-line truth about this provider: what (if anything) still
            // blocks the agent from using it.
            const status = !hasKey
              ? { text: "no key", tone: "text-muted-foreground" }
              : model
                ? { text: `key set · ${model}`, tone: "text-muted-foreground" }
                : { text: "key set — choose a model", tone: "text-amber-600 dark:text-amber-500" };
            return (
              <div key={p.kind}>
                <div className="flex items-center gap-3 px-4 py-3">
                  <input
                    type="radio"
                    name="default-provider"
                    className="accent-primary"
                    checked={isDefault}
                    onChange={() => setSettings((s) => ({ ...s, defaultProvider: p.kind }))}
                    aria-label={`Use ${p.label} as the default provider`}
                  />
                  <button
                    type="button"
                    aria-expanded={isOpen}
                    onClick={() => setExpanded(isOpen ? null : p.kind)}
                    className="flex flex-1 items-center justify-between gap-3 text-left"
                  >
                    <span className="text-sm font-medium">
                      {p.label}
                      {isDefault && (
                        <span className="ml-2 rounded-full bg-primary/10 px-2 py-0.5 text-xs font-normal text-primary">
                          default
                        </span>
                      )}
                    </span>
                    <span className={`flex items-center gap-2 text-xs ${status.tone}`}>
                      <span className="truncate font-mono">{status.text}</span>
                      <ChevronDown
                        className={`size-4 shrink-0 text-muted-foreground transition-transform${isOpen ? " rotate-180" : ""}`}
                        aria-hidden="true"
                      />
                    </span>
                  </button>
                </div>

                {isOpen && (
                  <div className="flex flex-col gap-3 border-t border-border bg-muted/20 px-4 py-4">
                    {p.needsBaseUrl && (
                      <label className="flex flex-col gap-1 text-sm">
                        <span className="text-xs text-muted-foreground">Base URL</span>
                        <TextInput
                          placeholder="https://openrouter.ai/api/v1"
                          value={settings.baseUrls[slug] ?? ""}
                          onValueChange={(v) => setBaseUrl(p.kind, v)}
                        />
                      </label>
                    )}

                    <div className="flex items-end gap-2">
                      <label className="flex flex-1 flex-col gap-1 text-sm">
                        <span className="text-xs text-muted-foreground">API key</span>
                        <TextInput
                          type="password"
                          placeholder={hasKey ? "•••• (stored) — enter a new key to replace" : "Paste API key"}
                          value={keyDrafts[p.kind] ?? ""}
                          onValueChange={(v) => setKeyDrafts((d) => ({ ...d, [p.kind]: v }))}
                        />
                      </label>
                      <Button onClick={() => saveKey(p.kind)} disabled={!keyDrafts[p.kind]?.trim()}>
                        Save key
                      </Button>
                      {hasKey && (
                        <Button
                          variant="ghost"
                          onClick={() => clearKey(p.kind)}
                          aria-label={`Remove ${p.label} key`}
                        >
                          <Trash2 className="size-4" aria-hidden="true" />
                        </Button>
                      )}
                    </div>

                    <div className="flex items-end gap-2">
                      <label className="flex flex-1 flex-col gap-1 text-sm">
                        <span className="text-xs text-muted-foreground">Model</span>
                        {list.length > 0 ? (
                          <select
                            className="h-9 rounded-md border border-border bg-background px-2"
                            value={model}
                            onChange={(e) => setModel(p.kind, e.target.value)}
                          >
                            <option value="">Choose a model…</option>
                            {list.map((m) => (
                              <option key={m.id} value={m.id}>
                                {m.displayName}
                              </option>
                            ))}
                          </select>
                        ) : (
                          <TextInput
                            placeholder="Model id (or fetch the list)"
                            value={model}
                            onValueChange={(v) => setModel(p.kind, v)}
                          />
                        )}
                      </label>
                      <Button
                        variant="ghost"
                        onClick={() => fetchModels(p.kind)}
                        disabled={!hasKey || busy === `models:${p.kind}`}
                        title={hasKey ? "Fetch available models" : "Add a key first"}
                      >
                        <RefreshCw
                          className={`size-4${busy === `models:${p.kind}` ? " animate-spin" : ""}`}
                          aria-hidden="true"
                        />
                        <span className="ml-1">Fetch models</span>
                      </Button>
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      <div>
        <Button onClick={saveSettings} disabled={busy === "settings"}>
          Save settings
        </Button>
      </div>

      <div className="flex flex-col gap-3 border-t border-border pt-6">
        <div>
          <h3 className="text-sm font-semibold">Coding agent CLIs</h3>
          <p className="text-xs text-muted-foreground">
            External agents detected on your PATH. Only installed ones can be selected in chat.
          </p>
        </div>
        {clis.map((a) => (
          <div
            key={a.kind}
            className="flex items-center justify-between rounded-lg border border-border px-4 py-3"
          >
            <span className="font-medium">{a.label}</span>
            {a.available ? (
              <span className="inline-flex items-center gap-1 rounded-full bg-green-500/10 px-2 py-0.5 text-xs text-green-600 dark:text-green-400">
                <Check className="size-3" aria-hidden="true" /> installed
              </span>
            ) : a.installUrl ? (
              <a
                href={a.installUrl}
                target="_blank"
                rel="noreferrer"
                className="text-xs text-muted-foreground underline"
              >
                not installed — how to install
              </a>
            ) : (
              <span className="text-xs text-muted-foreground">not installed</span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
