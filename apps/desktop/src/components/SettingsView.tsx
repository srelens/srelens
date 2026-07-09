import React, { useEffect, useRef, useState } from "react";
import {
  Boxes,
  Check,
  Download,
  LayoutPanelLeft,
  Monitor,
  Moon,
  Network,
  Palette,
  PanelLeft,
  PanelRight,
  RotateCcw,
  Sun,
  Timer,
  Upload,
  FilePlus2,
  X,
  ArrowDown,
  ArrowUp,
  GripVertical,
  ClipboardPaste,
  Plug,
  Trash2,
} from "lucide-react";
import {
  PageHeader,
  PageShell,
  SectionPanel,
  TextInput,
  Button,
  ConfirmDialog,
  THEME_OPTIONS,
  type Theme,
  type ThemeMode,
  type ThemeName,
} from "../ui";
import {
  ContextMenu,
  ContextMenuTrigger,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
} from "@/components/ui/context-menu";
import { listContexts, deleteContext, type ClusterContext } from "../lib/clusters";
import { notify } from "../lib/notify";
import {
  DEFAULT_WORKSPACE_LAYOUT,
  REQUEST_TIMEOUT,
  contextDisplayName,
  getRequestTimeoutSecs,
  loadUpdateChannel,
  saveUpdateChannel,
  type ContextLogo,
  type ContextProfiles,
  type UpdateChannel,
  type WorkspaceLayoutSettings,
  orderContexts,
} from "../lib/settings";
import { updateRequestTimeout } from "../lib/requestTimeout";
import { ContextAvatar, CONTEXT_LOGO_OPTIONS } from "./ContextAvatar";
import { McpSettingsSection } from "./McpSettingsSection";
import { pickKubeconfigFiles, savePastedKubeconfig } from "../lib/files";
import { checkForUpdate, installUpdate, type UpdateMeta } from "../lib/updater";
import { appVersion, relaunchApp } from "../transport/transport";

const MODE_OPTIONS: Array<{ mode: ThemeMode; label: string; description: string; icon: React.ElementType }> = [
  { mode: "dark", label: "Dark", description: "Low-light operational workspace", icon: Moon },
  { mode: "light", label: "Light", description: "Bright tables and resource browsing", icon: Sun },
  { mode: "system", label: "System", description: "Follow the OS appearance", icon: Monitor },
];

export type SettingsSection = "appearance" | "layout" | "kubernetes" | "contexts" | "mcp" | "updates";

type UpdatePhase =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "uptodate" }
  | { phase: "available"; update: UpdateMeta }
  | { phase: "downloading"; percent: number | null }
  | { phase: "ready" }
  | { phase: "error"; message: string };

const UPDATE_CHANNELS: Array<{ id: UpdateChannel; label: string; description: string }> = [
  { id: "stable", label: "Stable", description: "Released versions" },
  { id: "dev", label: "Dev", description: "Rolling pre-releases" },
];

const SETTINGS_SECTIONS: Array<{
  id: SettingsSection;
  label: string;
  description: string;
  icon: React.ElementType;
}> = [
  { id: "appearance", label: "Appearance", description: "Theme and display mode", icon: Palette },
  { id: "layout", label: "Layout", description: "Panel dimensions", icon: LayoutPanelLeft },
  { id: "kubernetes", label: "Kubernetes", description: "Workspace defaults", icon: Network },
  { id: "contexts", label: "Contexts", description: "Names, logos and colors", icon: Boxes },
  { id: "mcp", label: "MCP", description: "Agent access and client config", icon: Plug },
  { id: "updates", label: "Updates", description: "App version and updates", icon: Download },
];

const CONTEXT_COLORS = ["#2563eb", "#7c3aed", "#db2777", "#dc2626", "#ea580c", "#16a34a", "#0891b2", "#475569"];

export function SettingsView({
  theme,
  onThemeNameChange,
  onThemeModeChange,
  defaultNamespace,
  onDefaultNamespaceChange,
  layout,
  onLayoutChange,
  contextProfiles,
  onContextProfilesChange,
  kubeconfigFiles,
  onKubeconfigFilesChange,
  contextOrder,
  onContextOrderChange,
  initialSection = "appearance",
  contexts: passedContexts = null,
  contextsError = "",
  onDeleteContext,
}: {
  theme: Theme;
  onThemeNameChange: (name: ThemeName) => void;
  onThemeModeChange: (mode: ThemeMode) => void;
  defaultNamespace: string;
  onDefaultNamespaceChange: (namespace: string) => void;
  layout: WorkspaceLayoutSettings;
  onLayoutChange: (layout: WorkspaceLayoutSettings) => void;
  contextProfiles: ContextProfiles;
  onContextProfilesChange: (profiles: ContextProfiles) => void;
  kubeconfigFiles: string[];
  onKubeconfigFilesChange: (paths: string[]) => void;
  contextOrder: string[];
  onContextOrderChange: (order: string[]) => void;
  /** Section to open on mount (e.g. deep-linked from the update toast). */
  initialSection?: SettingsSection;
  contexts?: ClusterContext[] | null;
  contextsError?: string;
  onDeleteContext?: (name: string) => Promise<void>;
}) {
  const [section, setSection] = useState<SettingsSection>(initialSection);
  const [internalContexts, setInternalContexts] = useState<ClusterContext[] | null>(null);
  const [internalError, setInternalError] = useState("");
  const [contextQuery, setContextQuery] = useState("");
  const [selectedContextName, setSelectedContextName] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [logoError, setLogoError] = useState("");
  const [kubeconfigError, setKubeconfigError] = useState("");
  const [draggedContextName, setDraggedContextName] = useState<string | null>(null);
  const [dropTargetName, setDropTargetName] = useState<string | null>(null);
  const [pasteKubeconfigOpen, setPasteKubeconfigOpen] = useState(false);
  const [pastedKubeconfig, setPastedKubeconfig] = useState("");
  const [pastedKubeconfigName, setPastedKubeconfigName] = useState("");
  const [updateState, setUpdateState] = useState<UpdatePhase>({ phase: "idle" });
  const [updateChannel, setUpdateChannel] = useState<UpdateChannel>(() => loadUpdateChannel());
  const [currentVersion, setCurrentVersion] = useState("");
  const [requestTimeout, setRequestTimeout] = useState(() => getRequestTimeoutSecs());
  const draggedContextRef = useRef<string | null>(null);
  const dropTargetRef = useRef<string | null>(null);

  useEffect(() => {
    let active = true;
    appVersion()
      .then((version) => {
        if (active) setCurrentVersion(version);
      })
      .catch(() => {
        /* version display is cosmetic — never block settings on it */
      });
    return () => {
      active = false;
    };
  }, []);

  const switchUpdateChannel = (channel: UpdateChannel) => {
    setUpdateChannel(channel);
    saveUpdateChannel(channel);
    setUpdateState({ phase: "idle" });
  };

  const runUpdateCheck = async () => {
    setUpdateState({ phase: "checking" });
    try {
      const update = await checkForUpdate(updateChannel);
      setUpdateState(update ? { phase: "available", update } : { phase: "uptodate" });
    } catch (cause) {
      setUpdateState({ phase: "error", message: cause instanceof Error ? cause.message : String(cause) });
    }
  };

  const startInstall = async () => {
    setUpdateState({ phase: "downloading", percent: null });
    try {
      await installUpdate(updateChannel, (percent) => setUpdateState({ phase: "downloading", percent }));
      setUpdateState({ phase: "ready" });
    } catch (cause) {
      setUpdateState({ phase: "error", message: cause instanceof Error ? cause.message : String(cause) });
    }
  };

  useEffect(() => {
    if (passedContexts !== null) return;
    let active = true;
    void listContexts(kubeconfigFiles).then((outcome) => {
      if (!active) return;
      setInternalContexts(outcome.contexts ?? []);
      setSelectedContextName((current) => current ?? outcome.contexts?.[0]?.name ?? null);
      setInternalError(outcome.error ?? "");
    });
    return () => {
      active = false;
    };
  }, [kubeconfigFiles, passedContexts]);

  useEffect(() => {
    if (passedContexts !== null) {
      setSelectedContextName((current) => {
        if (current && passedContexts.some((c) => c.name === current)) {
          return current;
        }
        return passedContexts[0]?.name ?? null;
      });
    }
  }, [passedContexts]);

  const contexts = passedContexts !== null ? passedContexts : internalContexts;
  const contextError = passedContexts !== null ? contextsError : internalError;

  const confirmDeleteContext = async () => {
    const name = pendingDelete;
    if (!name) return;
    setDeleteBusy(true);
    try {
      if (onDeleteContext) {
        await onDeleteContext(name);
      } else {
        await deleteContext(name);
        resetContext(name);
        onContextOrderChange(contextOrder.filter((item) => item !== name));
        const outcome = await listContexts(kubeconfigFiles);
        setInternalContexts(outcome.contexts ?? []);
      }
      setSelectedContextName(null);
      setPendingDelete(null);
    } catch (e) {
      notify.error("Failed to remove context", String(e));
    } finally {
      setDeleteBusy(false);
    }
  };

  const updateContext = (name: string, patch: ContextProfiles[string]) => {
    onContextProfilesChange({
      ...contextProfiles,
      [name]: { ...contextProfiles[name], ...patch },
    });
  };

  const resetContext = (name: string) => {
    const next = { ...contextProfiles };
    delete next[name];
    onContextProfilesChange(next);
  };

  const orderedContexts = orderContexts(contexts ?? [], contextOrder);
  const filteredContexts = orderedContexts.filter((context) => {
    const query = contextQuery.trim().toLowerCase();
    if (!query) return true;
    return [context.name, context.cluster, contextDisplayName(context.name, contextProfiles[context.name])]
      .some((value) => value.toLowerCase().includes(query));
  });
  const selectedContext = orderedContexts.find((context) => context.name === selectedContextName) ?? orderedContexts[0] ?? null;
  const selectedProfile = selectedContext ? contextProfiles[selectedContext.name] ?? {} : {};

  const uploadLogo = (file?: File) => {
    setLogoError("");
    if (!file || !selectedContext) return;
    if (!['image/png', 'image/jpeg', 'image/webp', 'image/gif'].includes(file.type)) {
      setLogoError("Use a PNG, JPEG, WebP, or GIF image.");
      return;
    }
    if (file.size > 512 * 1024) {
      setLogoError("Logo must be smaller than 512 KB.");
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") {
        updateContext(selectedContext.name, { logo: "custom", logoUrl: reader.result });
      }
    };
    reader.onerror = () => setLogoError("Unable to read this image.");
    reader.readAsDataURL(file);
  };

  const addKubeconfigFiles = async () => {
    setKubeconfigError("");
    try {
      const selected = await pickKubeconfigFiles();
      if (selected.length) onKubeconfigFilesChange([...new Set([...kubeconfigFiles, ...selected])]);
    } catch (error) {
      setKubeconfigError(String(error));
    }
  };

  const addPastedKubeconfig = async () => {
    setKubeconfigError("");
    try {
      const path = await savePastedKubeconfig(pastedKubeconfig, pastedKubeconfigName.trim() || undefined);
      onKubeconfigFilesChange([...new Set([...kubeconfigFiles, path])]);
      setPastedKubeconfig("");
      setPastedKubeconfigName("");
      setPasteKubeconfigOpen(false);
    } catch (error) {
      setKubeconfigError(String(error));
    }
  };

  const moveContext = (name: string, offset: number) => {
    const order = orderedContexts.map((context) => context.name);
    const index = order.indexOf(name);
    const target = index + offset;
    if (index < 0 || target < 0 || target >= order.length) return;
    [order[index], order[target]] = [order[target], order[index]];
    onContextOrderChange(order);
  };

  const dropContextBefore = (sourceName: string, targetName: string) => {
    if (!sourceName || sourceName === targetName) return;
    const original = orderedContexts.map((context) => context.name);
    const sourceIndex = original.indexOf(sourceName);
    const targetIndex = original.indexOf(targetName);
    const order = original.filter((name) => name !== sourceName);
    const target = order.indexOf(targetName);
    const insertion = target < 0 ? order.length : target + (sourceIndex < targetIndex ? 1 : 0);
    order.splice(insertion, 0, sourceName);
    onContextOrderChange(order);
    setDraggedContextName(null);
    setDropTargetName(null);
  };

  const beginPointerContextDrag = (event: React.PointerEvent<HTMLSpanElement>, name: string) => {
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    draggedContextRef.current = name;
    dropTargetRef.current = null;
    setDraggedContextName(name);
    setDropTargetName(null);
  };

  const updatePointerContextDrag = (event: React.PointerEvent<HTMLSpanElement>) => {
    const source = draggedContextRef.current;
    if (!source) return;
    const target = document
      .elementFromPoint(event.clientX, event.clientY)
      ?.closest<HTMLElement>("[data-context-name]")
      ?.dataset.contextName;
    if (!target || target === source) return;
    dropTargetRef.current = target;
    setDropTargetName(target);
  };

  const finishPointerContextDrag = (event: React.PointerEvent<HTMLSpanElement>) => {
    const source = draggedContextRef.current;
    const target = dropTargetRef.current;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    draggedContextRef.current = null;
    dropTargetRef.current = null;
    setDraggedContextName(null);
    setDropTargetName(null);
    if (source && target) dropContextBefore(source, target);
  };

  return (
    <PageShell className="fl-settings-page">
      <PageHeader
        eyebrow="Settings"
        title="Workspace preferences"
        description="Configure appearance, layout, Kubernetes defaults, and context identity."
      />

      <div className="fl-settings-workspace">
        <nav className="fl-settings-nav" aria-label="Settings sections">
          {SETTINGS_SECTIONS.map(({ id, label, description, icon: Icon }) => (
            <button
              key={id}
              type="button"
              className={`fl-settings-nav__item${section === id ? " fl-settings-nav__item--active" : ""}`}
              onClick={() => setSection(id)}
              aria-current={section === id ? "page" : undefined}
            >
              <Icon aria-hidden="true" />
              <span>
                <strong>{label}</strong>
                <small>{description}</small>
              </span>
            </button>
          ))}
        </nav>

        <div className="fl-settings-content">
          {section === "appearance" && (
            <SectionPanel title="Appearance" description="Choose a palette and display mode. Changes apply immediately.">
              <div className="fl-settings-mode-grid" role="group" aria-label="Theme mode">
                {MODE_OPTIONS.map(({ mode, label, description, icon: Icon }) => (
                  <button
                    key={mode}
                    type="button"
                    className={`fl-settings-mode${theme.mode === mode ? " fl-settings-mode--active" : ""}`}
                    onClick={() => onThemeModeChange(mode)}
                    aria-pressed={theme.mode === mode}
                  >
                    <Icon aria-hidden="true" />
                    <span>
                      <strong>{label}</strong>
                      <small>{description}</small>
                    </span>
                    {theme.mode === mode && <Check aria-hidden="true" />}
                  </button>
                ))}
              </div>
              <div className="fl-settings-theme-grid" aria-label="Theme palette">
                {THEME_OPTIONS.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    className={`fl-settings-theme-card${theme.name === option.id ? " fl-settings-theme-card--active" : ""}`}
                    onClick={() => onThemeNameChange(option.id)}
                    aria-pressed={theme.name === option.id}
                  >
                    <span className="fl-settings-theme-card__preview" style={{ background: option.preview }} />
                    <span className="fl-settings-theme-card__copy">
                      <strong>{option.name}</strong>
                      <small>{option.description}</small>
                    </span>
                    {theme.name === option.id && <Check aria-hidden="true" />}
                  </button>
                ))}
              </div>
            </SectionPanel>
          )}

          {section === "layout" && (
            <SectionPanel
              title="Workspace layout"
              description="Choose default panel widths. Either panel can still be resized by dragging it."
            >
              <div className="fl-settings-width-grid">
                <label className="fl-settings-width-control">
                  <span className="fl-settings-width-control__header">
                    <PanelLeft aria-hidden="true" />
                    <span><strong>Left navigation</strong><small>Cluster resource sidebar</small></span>
                    <output>{layout.leftSidebarWidth}px</output>
                  </span>
                  <input
                    type="range"
                    min="160"
                    max="420"
                    step="4"
                    value={layout.leftSidebarWidth}
                    onChange={(event) => onLayoutChange({ ...layout, leftSidebarWidth: Number(event.target.value) })}
                    aria-label="Default left navigation width"
                  />
                </label>
                <label className="fl-settings-width-control">
                  <span className="fl-settings-width-control__header">
                    <PanelRight aria-hidden="true" />
                    <span><strong>Right details</strong><small>Resource detail drawer</small></span>
                    <output>{layout.rightSidebarWidth}px</output>
                  </span>
                  <input
                    type="range"
                    min="320"
                    max="960"
                    step="8"
                    value={layout.rightSidebarWidth}
                    onChange={(event) => onLayoutChange({ ...layout, rightSidebarWidth: Number(event.target.value) })}
                    aria-label="Default right detail width"
                  />
                </label>
              </div>
              <Button variant="ghost" size="sm" className="mt-3" onClick={() => onLayoutChange({ ...DEFAULT_WORKSPACE_LAYOUT })}>
                <RotateCcw data-icon="inline-start" />
                Restore layout defaults
              </Button>
            </SectionPanel>
          )}

          {section === "kubernetes" && (
            <SectionPanel
              title="Kubernetes defaults"
              description="New resource tabs remember each cluster's last namespace, then fall back to this value."
            >
              <label className="fl-settings-field">
                <span>Default namespace</span>
                <TextInput
                  value={defaultNamespace}
                  onValueChange={onDefaultNamespaceChange}
                  placeholder="All namespaces"
                  aria-label="Default namespace"
                />
              </label>

              <div className="fl-settings-width-grid">
                <label className="fl-settings-width-control">
                  <span className="fl-settings-width-control__header">
                    <Timer aria-hidden="true" />
                    <span>
                      <strong>Request timeout</strong>
                      <small>How long to wait for a cluster response. Raise it for large clusters.</small>
                    </span>
                    <output>{requestTimeout}s</output>
                  </span>
                  <input
                    type="range"
                    min={REQUEST_TIMEOUT.MIN}
                    max={REQUEST_TIMEOUT.MAX}
                    step="1"
                    value={requestTimeout}
                    onChange={(event) => {
                      const secs = Number(event.target.value);
                      setRequestTimeout(secs);
                      void updateRequestTimeout(secs);
                    }}
                    aria-label="Cluster request timeout in seconds"
                  />
                </label>
              </div>
            </SectionPanel>
          )}

          {section === "contexts" && (
            <SectionPanel
              className="fl-settings-context-panel"
              title="Context management"
              description="Create a recognizable identity for every cluster without changing kubeconfig."
            >
              <div className="fl-kubeconfig-sources">
                <div>
                  <span>
                    <strong>Kubeconfig sources</strong>
                    <small>The default kubeconfig is loaded first; additional files are merged in order.</small>
                  </span>
                  <span className="fl-kubeconfig-sources__actions">
                    <Button variant="outline" size="sm" onClick={() => setPasteKubeconfigOpen((open) => !open)}>
                      <ClipboardPaste data-icon="inline-start" /> Paste
                    </Button>
                    <Button variant="outline" size="sm" onClick={() => void addKubeconfigFiles()}>
                      <FilePlus2 data-icon="inline-start" /> Add files
                    </Button>
                  </span>
                </div>
                <div className="fl-kubeconfig-sources__files">
                  <span className="is-default">Default / KUBECONFIG</span>
                  {kubeconfigFiles.map((path) => (
                    <span key={path} title={path}>
                      <code>{path.split(/[\\/]/).at(-1) || path}</code>
                      <button
                        type="button"
                        onClick={() => onKubeconfigFilesChange(kubeconfigFiles.filter((item) => item !== path))}
                        aria-label={`Remove kubeconfig ${path}`}
                      >
                        <X aria-hidden="true" />
                      </button>
                    </span>
                  ))}
                </div>
                {pasteKubeconfigOpen && (
                  <div className="fl-kubeconfig-paste">
                    <div>
                      <label>
                        <span>Name</span>
                        <TextInput
                          value={pastedKubeconfigName}
                          onValueChange={setPastedKubeconfigName}
                          placeholder="Team or environment"
                          aria-label="Pasted kubeconfig name"
                        />
                      </label>
                      <span>Saved securely in the srelens app configuration directory.</span>
                    </div>
                    <textarea
                      value={pastedKubeconfig}
                      onChange={(event) => setPastedKubeconfig(event.target.value)}
                      placeholder="Paste kubeconfig YAML here…"
                      aria-label="Kubeconfig YAML"
                      spellCheck={false}
                    />
                    <footer>
                      <Button variant="ghost" size="sm" onClick={() => setPasteKubeconfigOpen(false)}>Cancel</Button>
                      <Button size="sm" disabled={!pastedKubeconfig.trim()} onClick={() => void addPastedKubeconfig()}>
                        Add kubeconfig
                      </Button>
                    </footer>
                  </div>
                )}
                {kubeconfigError && <p role="alert">{kubeconfigError}</p>}
              </div>
              {contexts === null ? (
                <p className="fl-settings-context-state">Reading kubeconfig contexts…</p>
              ) : contextError ? (
                <p className="fl-settings-context-state">Unable to load kubeconfig contexts.</p>
              ) : contexts.length === 0 ? (
                <p className="fl-settings-context-state">No contexts are available.</p>
              ) : (
                <div className="fl-context-manager">
                  <aside className="fl-context-manager__list">
                    <TextInput
                      value={contextQuery}
                      onValueChange={setContextQuery}
                      type="search"
                      placeholder="Find a context…"
                      aria-label="Find a context"
                    />
                    <div>
                      {filteredContexts.map((context) => {
                        const profile = contextProfiles[context.name] ?? {};
                        return (
                          <ContextMenu key={context.name}>
                            <ContextMenuTrigger asChild>
                              <button
                                type="button"
                                className={[
                                  selectedContext?.name === context.name ? "is-active" : "",
                                  draggedContextName === context.name ? "is-dragging" : "",
                                  dropTargetName === context.name ? "is-drop-target" : "",
                                ].filter(Boolean).join(" ")}
                                data-context-name={context.name}
                                onClick={() => setSelectedContextName(context.name)}
                              >
                                <ContextAvatar context={context.name} profile={profile} className="fl-settings-context-avatar" />
                                <span>
                                  <strong>{contextDisplayName(context.name, profile)}</strong>
                                  <small>{context.name}</small>
                                </span>
                                {context.isCurrent && <i title="Current context" />}
                                <span
                                  className="fl-context-manager__grip"
                                  title="Drag to reorder"
                                  onPointerDown={(event) => beginPointerContextDrag(event, context.name)}
                                  onPointerMove={updatePointerContextDrag}
                                  onPointerUp={finishPointerContextDrag}
                                  onPointerCancel={finishPointerContextDrag}
                                >
                                  <GripVertical aria-hidden="true" />
                                </span>
                              </button>
                            </ContextMenuTrigger>
                            <ContextMenuContent>
                              <ContextMenuItem onSelect={() => resetContext(context.name)}>Reset identity</ContextMenuItem>
                              <ContextMenuSeparator />
                              <ContextMenuItem variant="destructive" onSelect={() => setPendingDelete(context.name)}>
                                Remove context
                              </ContextMenuItem>
                            </ContextMenuContent>
                          </ContextMenu>
                        );
                      })}
                      {filteredContexts.length === 0 && <p>No matching contexts</p>}
                    </div>
                  </aside>

                  {selectedContext && (
                    <article className="fl-context-editor">
                      <header>
                        <ContextAvatar
                          context={selectedContext.name}
                          profile={selectedProfile}
                          className="fl-context-editor__avatar"
                        />
                        <span>
                          <small>Context identity</small>
                          <h3>{contextDisplayName(selectedContext.name, selectedProfile)}</h3>
                          <code>{selectedContext.name}</code>
                        </span>
                        <div className="fl-context-editor__order">
                          {selectedContext.isCurrent && <span className="fl-settings-context-current">Current</span>}
                          <button
                            type="button"
                            onClick={() => moveContext(selectedContext.name, -1)}
                            disabled={orderedContexts[0]?.name === selectedContext.name}
                            aria-label={`Move ${selectedContext.name} up`}
                            title="Move up"
                          ><ArrowUp aria-hidden="true" /></button>
                          <button
                            type="button"
                            onClick={() => moveContext(selectedContext.name, 1)}
                            disabled={orderedContexts.at(-1)?.name === selectedContext.name}
                            aria-label={`Move ${selectedContext.name} down`}
                            title="Move down"
                          ><ArrowDown aria-hidden="true" /></button>
                        </div>
                      </header>

                      <section className="fl-context-editor__section">
                        <div className="fl-context-editor__heading">
                          <strong>Name and label</strong>
                          <small>Used in tabs, navigation, and the cluster switcher.</small>
                        </div>
                        <div className="fl-context-editor__identity">
                          <label>
                            <span>Display name</span>
                            <TextInput
                              value={selectedProfile.displayName ?? ""}
                              onValueChange={(displayName) => updateContext(selectedContext.name, { displayName })}
                              placeholder={selectedContext.name}
                              aria-label={`Display name for ${selectedContext.name}`}
                            />
                          </label>
                          <label>
                            <span>Short label</span>
                            <TextInput
                              value={selectedProfile.shortName ?? ""}
                              onValueChange={(shortName) => updateContext(selectedContext.name, { shortName: shortName.slice(0, 3) })}
                              placeholder="2–3 characters"
                              aria-label={`Short label for ${selectedContext.name}`}
                            />
                          </label>
                        </div>
                      </section>

                      <section className="fl-context-editor__section">
                        <div className="fl-context-editor__heading">
                          <strong>Logo</strong>
                          <small>Choose a symbol for the cluster hotbar.</small>
                        </div>
                        <div className="fl-context-editor__logos" role="group" aria-label={`Logo for ${selectedContext.name}`}>
                          {CONTEXT_LOGO_OPTIONS.map((option) => (
                            <button
                              key={option.value}
                              type="button"
                              className={(selectedProfile.logo ?? "initials") === option.value ? "is-active" : ""}
                              onClick={() => updateContext(selectedContext.name, { logo: option.value as ContextLogo })}
                              aria-pressed={(selectedProfile.logo ?? "initials") === option.value}
                            >
                              <ContextAvatar
                                context={selectedContext.name}
                                profile={{ ...selectedProfile, logo: option.value }}
                                className="fl-context-logo-preview"
                                showShortName={false}
                              />
                              <span>{option.label}</span>
                            </button>
                          ))}
                        </div>
                        {(selectedProfile.logo ?? "initials") === "custom" && (
                          <div className="fl-context-editor__custom-logo">
                            <label>
                              <span>Image URL</span>
                              <TextInput
                                value={selectedProfile.logoUrl?.startsWith("data:") ? "" : selectedProfile.logoUrl ?? ""}
                                onValueChange={(logoUrl) => {
                                  setLogoError("");
                                  updateContext(selectedContext.name, { logo: "custom", logoUrl });
                                }}
                                placeholder="https://example.com/cluster.png"
                                aria-label={`Custom logo URL for ${selectedContext.name}`}
                              />
                            </label>
                            <span className="fl-context-editor__custom-logo-divider">or</span>
                            <label className="fl-context-editor__upload">
                              <Upload aria-hidden="true" />
                              <span>{selectedProfile.logoUrl?.startsWith("data:") ? "Replace image" : "Upload image"}</span>
                              <input
                                type="file"
                                accept="image/png,image/jpeg,image/webp,image/gif"
                                onChange={(event) => uploadLogo(event.target.files?.[0])}
                                aria-label={`Upload custom logo for ${selectedContext.name}`}
                              />
                            </label>
                            {selectedProfile.logoUrl && (
                              <Button
                                variant="ghost"
                                size="sm"
                                onClick={() => updateContext(selectedContext.name, { logoUrl: "" })}
                              >
                                Remove
                              </Button>
                            )}
                            {logoError && <p role="alert">{logoError}</p>}
                          </div>
                        )}
                      </section>

                      <section className="fl-context-editor__section">
                        <div className="fl-context-editor__heading">
                          <strong>Color</strong>
                          <small>Use color to distinguish environments at a glance.</small>
                        </div>
                        <div className="fl-context-editor__colors">
                          {CONTEXT_COLORS.map((color) => (
                            <button
                              key={color}
                              type="button"
                              className={selectedProfile.color === color ? "is-active" : ""}
                              style={{ background: color }}
                              onClick={() => updateContext(selectedContext.name, { color })}
                              aria-label={`Set ${selectedContext.name} color to ${color}`}
                            />
                          ))}
                          <label title="Custom color">
                            <input
                              type="color"
                              value={selectedProfile.color ?? "#3b82f6"}
                              onChange={(event) => updateContext(selectedContext.name, { color: event.target.value })}
                              aria-label={`Custom color for ${selectedContext.name}`}
                            />
                          </label>
                          <code>{selectedProfile.color ?? "Automatic"}</code>
                        </div>
                      </section>

                      <footer>
                        <span>
                          <small>Cluster</small><code>{selectedContext.cluster}</code>
                          <small>Server</small><code>{selectedContext.server || "Not available"}</code>
                        </span>
                        <Button variant="ghost" size="sm" onClick={() => resetContext(selectedContext.name)}>
                          <RotateCcw data-icon="inline-start" /> Reset identity
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="hover:text-destructive hover:bg-destructive/10 text-muted-foreground"
                          onClick={() => setPendingDelete(selectedContext.name)}
                        >
                          <Trash2 data-icon="inline-start" /> Remove context
                        </Button>
                      </footer>
                    </article>
                  )}
                </div>
              )}
            </SectionPanel>
          )}

          {section === "mcp" && (
            <SectionPanel
              title="MCP"
              description="Expose srelens to agents and MCP clients, and get ready-to-paste client config."
            >
              <McpSettingsSection />
            </SectionPanel>
          )}

          {section === "updates" && (
            <SectionPanel title="Updates" description="Check for and install new versions of srelens.">
              <div className="fl-settings-update">
                <div className="fl-settings-update__version">
                  <small>Current version</small>
                  <code>{currentVersion || "unknown"}</code>
                </div>

                <div className="fl-settings-update__channels" role="group" aria-label="Update channel">
                  {UPDATE_CHANNELS.map(({ id, label, description }) => (
                    <button
                      key={id}
                      type="button"
                      className={`fl-settings-mode${updateChannel === id ? " fl-settings-mode--active" : ""}`}
                      onClick={() => switchUpdateChannel(id)}
                      aria-pressed={updateChannel === id}
                    >
                      <span>
                        <strong>{label}</strong>
                        <small>{description}</small>
                      </span>
                      {updateChannel === id && <Check aria-hidden="true" />}
                    </button>
                  ))}
                </div>

                {(updateState.phase === "idle" ||
                  updateState.phase === "checking" ||
                  updateState.phase === "uptodate" ||
                  updateState.phase === "error") && (
                  <Button onClick={() => void runUpdateCheck()} disabled={updateState.phase === "checking"}>
                    {updateState.phase === "checking" ? "Checking…" : "Check for updates"}
                  </Button>
                )}

                {updateState.phase === "uptodate" && <p className="fl-settings-update__status">srelens is up to date.</p>}
                {updateState.phase === "error" && (
                  <p className="fl-settings-update__status" role="alert">{updateState.message}</p>
                )}

                {updateState.phase === "available" && (
                  <div className="fl-settings-update__offer">
                    <p>
                      Version <strong>{updateState.update.version}</strong> is available on the {updateChannel}{" "}
                      channel.
                    </p>
                    {updateState.update.notes && (
                      <pre className="fl-settings-update__notes">{updateState.update.notes}</pre>
                    )}
                    <Button onClick={() => void startInstall()}>
                      <Download data-icon="inline-start" /> Download &amp; install
                    </Button>
                  </div>
                )}

                {updateState.phase === "downloading" && (
                  <p className="fl-settings-update__status" role="status">
                    Downloading{updateState.percent != null ? ` — ${updateState.percent}%` : "…"}
                  </p>
                )}

                {updateState.phase === "ready" && (
                  <div className="fl-settings-update__offer">
                    <p>Update installed. Restart to finish.</p>
                    <Button onClick={() => void relaunchApp()}>
                      <RotateCcw data-icon="inline-start" /> Restart srelens
                    </Button>
                  </div>
                )}
              </div>
            </SectionPanel>
          )}
        </div>
      </div>

      {pendingDelete && (
        <ConfirmDialog
          title="Remove context?"
          message={
            <p style={{ marginTop: 0 }}>
              Remove <code>{pendingDelete}</code> from its kubeconfig file? This modifies the kubeconfig on disk.
            </p>
          }
          confirmLabel="Remove"
          danger
          busy={deleteBusy}
          onConfirm={() => void confirmDeleteContext()}
          onCancel={() => setPendingDelete(null)}
        />
      )}
    </PageShell>
  );
}
