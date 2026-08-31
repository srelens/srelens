import { useEffect, useState } from "react";
import {
  listAgents,
  llmClearKey,
  llmGetSettings,
  llmKeyStatus,
  llmListModels,
  llmSetKey,
  llmSetSettings,
  PROVIDERS,
  providerSlug,
  type AgentInfo,
  type LlmSettings,
  type ModelInfo,
  type ProviderKind,
} from "@srelens/core";
import { Alert, Badge, Button, Panel, RawError, Select, TextInput } from "@srelens/ui-kit";
import { LOADING, type Read } from "../../lib/read";

/**
 * §23's Settings pane for the agent's own credentials: an API key per LLM
 * provider (never read back, only whether one is set), the default provider
 * and model srelens's native agent uses, and the external coding-agent CLIs
 * it can drive over MCP.
 *
 * **Failures here do NOT go through `errorCopy.tsx`'s `FailureAlert` /
 * `friendly()` / `describeError`.** Every branch of `describeError` classifies
 * a *cluster* failure — a 401 renders "The cluster rejected your credentials
 * … refresh your kubeconfig credentials", which is wrong advice for a
 * provider API key that was refused, and is issue #383's exact shape. This
 * pane talks to LLM providers and local CLIs, not to a cluster, so a failure
 * is rendered directly with the kit's `Alert` plus `RawError`: a title and one
 * sentence this pane writes itself, naming the operation and the provider,
 * with the original string one disclosure away underneath — same shape as
 * everywhere else, without borrowing cluster vocabulary to get there.
 *
 * **Three independent reads, three independent three-state unions.** Provider
 * settings (`llmGetSettings`), which providers have a key (`llmKeyStatus`) and
 * the CLI inventory (`listAgents`) are read once at mount and each kept as
 * `{ loading } | { error } | { ready, value }` rather than collapsed onto a
 * default array or object — a `[]` or `{}` used as the "not read yet" value
 * would be indistinguishable from "read, and it's empty", and this plan has
 * found that exact collapse five times already. The provider list is only
 * drawn once BOTH the settings and the key-status reads have landed: showing
 * a provider row's "no key" status from a `keyStatus` that hasn't resolved
 * would assert an absence this pane does not yet know, and showing model
 * settings from a `llmGetSettings` that failed would draw a default provider
 * nobody chose.
 *
 * **The key is never read back.** `llmSetKey` and `llmClearKey` both resolve
 * to nothing; only `llmKeyStatus`'s list of provider kinds is ever displayed,
 * and the field a reader types a key into is cleared to `""` on a successful
 * save rather than left holding what was just sent. Nothing in this pane
 * reconstructs or previews the stored value.
 */

const DEFAULT_SETTINGS: LlmSettings = {
  defaultProvider: "anthropic",
  models: {},
  baseUrls: {},
  maxTokens: 4096,
};

/** The sentence this pane writes itself for a failed model fetch — see the
 * file comment on why this does not go through `describeError`. */
function modelFetchTitle(label: string): string {
  return `Could not fetch models from ${label}`;
}

export function AgentPane() {
  const [settingsRead, setSettingsRead] = useState<Read<LlmSettings>>(LOADING);
  const [keyStatusRead, setKeyStatusRead] = useState<Read<ProviderKind[]>>(LOADING);
  const [agentsRead, setAgentsRead] = useState<Read<AgentInfo[]>>(LOADING);

  const [settings, setSettings] = useState<LlmSettings>(DEFAULT_SETTINGS);
  // Which provider row is open — one at a time, starting on the default once
  // settings load, since that's the one the agent actually uses.
  const [expanded, setExpanded] = useState<ProviderKind | null>(null);
  const [keyDrafts, setKeyDrafts] = useState<Partial<Record<ProviderKind, string>>>({});
  const [models, setModels] = useState<Partial<Record<ProviderKind, ModelInfo[]>>>({});
  const [modelFetchError, setModelFetchError] = useState<Partial<Record<ProviderKind, unknown>>>({});
  const [keyActionError, setKeyActionError] = useState<Partial<Record<ProviderKind, unknown>>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<unknown>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let cancelled = false;
    llmGetSettings()
      .then((s) => {
        if (cancelled) return;
        const merged = { ...DEFAULT_SETTINGS, ...s };
        setSettingsRead({ kind: "ready", value: merged });
        setSettings(merged);
        setExpanded(merged.defaultProvider);
      })
      .catch((e) => {
        if (!cancelled) setSettingsRead({ kind: "error", error: e });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  function refreshKeyStatus() {
    return llmKeyStatus()
      .then((v) => setKeyStatusRead({ kind: "ready", value: v }))
      .catch((e) => setKeyStatusRead({ kind: "error", error: e }));
  }

  useEffect(() => {
    let cancelled = false;
    llmKeyStatus()
      .then((v) => {
        if (!cancelled) setKeyStatusRead({ kind: "ready", value: v });
      })
      .catch((e) => {
        if (!cancelled) setKeyStatusRead({ kind: "error", error: e });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    listAgents()
      .then((v) => {
        // The native agent has its own section above (provider keys); this
        // list is the external CLIs it can hand a turn to instead.
        if (!cancelled) setAgentsRead({ kind: "ready", value: v.filter((a) => a.kind !== "srelens") });
      })
      .catch((e) => {
        if (!cancelled) setAgentsRead({ kind: "error", error: e });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  function editSettings(update: (s: LlmSettings) => LlmSettings) {
    setSettings(update);
    // A further edit invalidates whatever "Saved." claimed about a draft that
    // no longer matches what's on screen.
    setSaved(false);
  }

  function setModel(provider: ProviderKind, model: string) {
    editSettings((s) => ({ ...s, models: { ...s.models, [providerSlug(provider)]: model } }));
  }

  function setBaseUrl(provider: ProviderKind, url: string) {
    editSettings((s) => ({ ...s, baseUrls: { ...s.baseUrls, [providerSlug(provider)]: url } }));
  }

  async function saveKey(provider: ProviderKind) {
    const draft = keyDrafts[provider]?.trim();
    if (!draft) return;
    setBusy(`key:${provider}`);
    setKeyActionError((m) => ({ ...m, [provider]: null }));
    try {
      await llmSetKey(provider, draft);
      // Cleared, not kept: the field never holds what was just sent, and
      // nothing here reconstructs the stored value to show in its place.
      setKeyDrafts((d) => ({ ...d, [provider]: "" }));
      await refreshKeyStatus();
    } catch (e) {
      setKeyActionError((m) => ({ ...m, [provider]: e }));
    } finally {
      setBusy(null);
    }
  }

  async function clearKey(provider: ProviderKind) {
    setBusy(`key:${provider}`);
    setKeyActionError((m) => ({ ...m, [provider]: null }));
    try {
      await llmClearKey(provider);
      await refreshKeyStatus();
    } catch (e) {
      setKeyActionError((m) => ({ ...m, [provider]: e }));
    } finally {
      setBusy(null);
    }
  }

  async function fetchModels(provider: ProviderKind) {
    setBusy(`models:${provider}`);
    setModelFetchError((m) => ({ ...m, [provider]: null }));
    try {
      const list = await llmListModels(provider, settings.baseUrls[providerSlug(provider)]);
      setModels((m) => ({ ...m, [provider]: list }));
    } catch (e) {
      setModelFetchError((m) => ({ ...m, [provider]: e }));
    } finally {
      setBusy(null);
    }
  }

  async function saveSettings() {
    setBusy("settings");
    setSaveError(null);
    try {
      await llmSetSettings(settings);
      setSaved(true);
    } catch (e) {
      setSaveError(e);
    } finally {
      setBusy(null);
    }
  }

  // See the file comment: neither the provider rows nor a claim about any of
  // them is drawn until BOTH reads that feed them have actually landed.
  const stillLoadingProviders = settingsRead.kind === "loading" || keyStatusRead.kind === "loading";
  const keyed = keyStatusRead.kind === "ready" ? keyStatusRead.value : null;

  return (
    <div className="flex flex-col gap-4">
      <Panel
        title="Providers"
        description="srelens's own agent talks directly to a provider with your API key. The key is written to the OS keychain and never read back — only whether one is set is shown here."
      >
        {stillLoadingProviders ? (
          <p className="text-[0.75rem] text-muted">Checking configured providers…</p>
        ) : (
          <>
            {settingsRead.kind === "error" && (
              <Alert tone="sev" title="Provider settings could not be loaded">
                The default provider and per-provider models can&apos;t be shown until this loads.
                <RawError text={String(settingsRead.error)} className="mt-1" />
              </Alert>
            )}
            {keyStatusRead.kind === "error" && (
              <Alert tone="sev" title="Which providers have a key could not be checked">
                Nothing here changed the keychain; this pane just couldn&apos;t read its status.
                <RawError text={String(keyStatusRead.error)} className="mt-1" />
              </Alert>
            )}
            {settingsRead.kind === "ready" && keyStatusRead.kind === "ready" && (
              <>
                <div className="rounded-md border border-rule">
                  {PROVIDERS.map((p) => {
                    const slug = providerSlug(p.kind);
                    const hasKey = keyed?.includes(p.kind) ?? false;
                    const list = models[p.kind] ?? [];
                    const model = settings.models[slug] ?? "";
                    const isDefault = settings.defaultProvider === p.kind;
                    const isOpen = expanded === p.kind;
                    const fetchErr = modelFetchError[p.kind];
                    const keyErr = keyActionError[p.kind];
                    return (
                      <div
                        key={p.kind}
                        data-testid={`provider-row-${p.kind}`}
                        className="border-b border-rule last:border-b-0"
                      >
                        <div className="flex items-center gap-3 px-3 py-2">
                          <input
                            type="radio"
                            name="default-provider"
                            checked={isDefault}
                            onChange={() => editSettings((s) => ({ ...s, defaultProvider: p.kind }))}
                            aria-label={`Use ${p.label} as the default provider`}
                          />
                          <button
                            type="button"
                            aria-expanded={isOpen}
                            onClick={() => setExpanded(isOpen ? null : p.kind)}
                            className="flex min-w-0 flex-1 items-center justify-between gap-3 text-left"
                          >
                            <span className="flex shrink-0 items-center gap-2 text-[0.8125rem] font-medium">
                              {p.label}
                              {isDefault && <Badge tone="accent">default</Badge>}
                            </span>
                            {/* M13 (controller finding): `model` is a string the
                                reader types free-hand below ("Model id (or fetch
                                the list)") — this is the branch's only file with
                                no shrink discipline (0 `min-w-0`, 0 `truncate`
                                across 12 flex rows), and a long id here has
                                nowhere to go but sideways in a fixed-width pane. */}
                            <span className="min-w-0 truncate text-[0.75rem] text-muted">
                              {!hasKey
                                ? "no key"
                                : model
                                  ? `key set · ${model}`
                                  : "key set — choose a model"}
                            </span>
                          </button>
                        </div>

                        {isOpen && (
                          <div className="flex flex-col gap-2 border-t border-rule bg-canvas-sunken px-3 py-3">
                            {p.needsBaseUrl && (
                              <label className="flex flex-col gap-1 text-[0.75rem]">
                                <span className="text-muted">Base URL</span>
                                <TextInput
                                  value={settings.baseUrls[slug] ?? ""}
                                  onValueChange={(v) => setBaseUrl(p.kind, v)}
                                  placeholder="https://openrouter.ai/api/v1"
                                  aria-label={`${p.label} base URL`}
                                />
                              </label>
                            )}

                            <div className="flex items-end gap-2">
                              <label className="flex flex-1 flex-col gap-1 text-[0.75rem]">
                                <span className="text-muted">API key</span>
                                <TextInput
                                  type="password"
                                  value={keyDrafts[p.kind] ?? ""}
                                  onValueChange={(v) => setKeyDrafts((d) => ({ ...d, [p.kind]: v }))}
                                  placeholder={hasKey ? "Enter a new key to replace it" : "Paste API key"}
                                  aria-label={`${p.label} API key`}
                                />
                              </label>
                              <Button
                                size="sm"
                                disabled={!keyDrafts[p.kind]?.trim() || busy === `key:${p.kind}`}
                                onClick={() => void saveKey(p.kind)}
                              >
                                Save key
                              </Button>
                              {hasKey && (
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  disabled={busy === `key:${p.kind}`}
                                  onClick={() => void clearKey(p.kind)}
                                >
                                  Remove key
                                </Button>
                              )}
                            </div>
                            {keyErr != null && (
                              <Alert tone="sev" title={`${p.label}'s key could not be updated`}>
                                <RawError text={String(keyErr)} />
                              </Alert>
                            )}

                            <div className="flex items-end gap-2">
                              <label className="flex flex-1 flex-col gap-1 text-[0.75rem]">
                                <span className="text-muted">Model</span>
                                {list.length > 0 ? (
                                  <Select
                                    value={model}
                                    onValueChange={(v) => setModel(p.kind, v)}
                                    options={list.map((m) => ({ value: m.id, label: m.displayName }))}
                                    placeholder="Choose a model…"
                                    aria-label={`${p.label} model`}
                                  />
                                ) : (
                                  <TextInput
                                    value={model}
                                    onValueChange={(v) => setModel(p.kind, v)}
                                    placeholder="Model id (or fetch the list)"
                                    aria-label={`${p.label} model id`}
                                  />
                                )}
                              </label>
                              <Button
                                variant="ghost"
                                size="sm"
                                disabled={!hasKey || busy === `models:${p.kind}`}
                                onClick={() => void fetchModels(p.kind)}
                              >
                                Fetch models
                              </Button>
                            </div>
                            {fetchErr != null && (
                              <Alert tone="sev" title={modelFetchTitle(p.label)}>
                                <RawError text={String(fetchErr)} />
                              </Alert>
                            )}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>

                <div className="mt-3 flex items-center gap-2">
                  <Button size="sm" disabled={busy === "settings"} onClick={() => void saveSettings()}>
                    Save settings
                  </Button>
                  {saved && <span className="text-[0.75rem] text-muted">Saved.</span>}
                </div>
                {saveError != null && (
                  <Alert tone="sev" title="Provider settings could not be saved" className="mt-2">
                    <RawError text={String(saveError)} />
                  </Alert>
                )}
              </>
            )}
          </>
        )}
      </Panel>

      <Panel
        title="Agent CLIs"
        description="External coding-agent CLIs srelens can drive over MCP. Only the ones found on PATH are selectable in chat."
      >
        {agentsRead.kind === "loading" && (
          <p className="text-[0.75rem] text-muted">Checking installed agent CLIs…</p>
        )}
        {agentsRead.kind === "error" && (
          <Alert tone="sev" title="Installed agent CLIs could not be checked">
            <RawError text={String(agentsRead.error)} />
          </Alert>
        )}
        {agentsRead.kind === "ready" && (
          <div className="rounded-md border border-rule">
            {agentsRead.value.map((a) => (
              <div
                key={a.kind}
                className="flex min-w-0 items-center justify-between gap-3 border-b border-rule px-3 py-2 last:border-b-0"
              >
                <span className="min-w-0 flex-1 truncate text-[0.8125rem] font-medium">{a.label}</span>
                {a.available ? (
                  <span className="min-w-0 shrink-0 truncate">
                    <Badge tone="ok">{a.version ?? "installed"}</Badge>
                  </span>
                ) : a.installUrl ? (
                  <a
                    href={a.installUrl}
                    target="_blank"
                    rel="noreferrer"
                    className="min-w-0 shrink-0 truncate text-[0.75rem] text-muted underline"
                  >
                    not installed — install {a.label}
                  </a>
                ) : (
                  <span className="min-w-0 shrink-0 truncate text-[0.75rem] text-muted">not installed</span>
                )}
              </div>
            ))}
          </div>
        )}
      </Panel>
    </div>
  );
}
