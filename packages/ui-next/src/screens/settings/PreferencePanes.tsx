import { useState } from "react";
import {
  DEFAULT_WORKSPACE_LAYOUT, getDefaultNamespace, getRequestTimeoutSecs, isTauri, loadRestoreSession,
  REQUEST_TIMEOUT, saveRestoreSession, updateRequestTimeout,
} from "@srelens/core";
import { Button, Field, Panel, Switch, TextInput } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";
import { DEFAULT_PEEK_WIDTH, MIN_PEEK_WIDTH, MAX_PEEK_WIDTH, savePeekWidth, usePeekWidth } from "../../lib/peekWidth";
import { MIN_NAVIGATION_WIDTH, MAX_NAVIGATION_WIDTH, saveNavigationWidth, useNavigationWidth } from "../../lib/navigationWidth";
import { openTab } from "../../lib/tabsStore";
import { setNamespaceDefault } from "../../lib/workspace";

export function WorkspacePane() {
  const [restore, setRestore] = useState(loadRestoreSession);
  const width = usePeekWidth();
  const navigationWidth = useNavigationWidth();
  return (
    <div className="flex flex-col gap-4">
      <Panel title="Session">
        <Switch label="Reopen tabs on launch" hint="Return to your saved workspace when srelens starts."
          on={restore} onChange={(value) => { saveRestoreSession(value); setRestore(value); }} />
      </Panel>
      <Panel title="Workspace layout" description="Panels can also be resized by dragging their divider.">
        <Field label="Left navigation width" hint={`${navigationWidth}px`}>
          <input type="range" className="w-full" min={MIN_NAVIGATION_WIDTH} max={MAX_NAVIGATION_WIDTH} step={4}
            value={navigationWidth} onChange={(event) => saveNavigationWidth(Number(event.target.value))} />
        </Field>
        <Field label="Resource detail width" hint={`${width}px — the pane fits the available space on smaller windows.`}>
          <input type="range" className="w-full" min={MIN_PEEK_WIDTH} max={MAX_PEEK_WIDTH} step={4}
            value={width} onChange={(event) => savePeekWidth(Number(event.target.value))} />
        </Field>
        <Button variant="secondary" onClick={() => { savePeekWidth(DEFAULT_PEEK_WIDTH); saveNavigationWidth(DEFAULT_WORKSPACE_LAYOUT.leftSidebarWidth); }}>Restore layout defaults</Button>
      </Panel>
    </div>
  );
}

export function KubernetesPane() {
  const [namespace, setNamespace] = useState(getDefaultNamespace);
  const [savedNamespace, setSavedNamespace] = useState(getDefaultNamespace);
  const [timeout, setTimeout] = useState(() => String(getRequestTimeoutSecs()));
  const [savedTimeout, setSavedTimeout] = useState(getRequestTimeoutSecs);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const seconds = Number(timeout);
  const valid = timeout.trim() !== "" && Number.isInteger(seconds) && seconds >= REQUEST_TIMEOUT.MIN && seconds <= REQUEST_TIMEOUT.MAX;
  const validNamespace = namespace === "" || (namespace.length <= 63 && /^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/.test(namespace));
  async function saveTimeout() {
    setBusy(true); setError(null);
    try {
      const applied = await updateRequestTimeout(seconds);
      setSavedTimeout(applied); setTimeout(String(applied));
    } catch (cause) { setError(cause); }
    finally { setBusy(false); }
  }
  return (
    <div className="flex flex-col gap-4">
      <Panel title="Kubernetes defaults" description="Used for clusters with no remembered namespace selection.">
        <Field label="Default namespace" hint="Leave empty for all namespaces. Existing selections stay as you chose them."
          error={validNamespace ? undefined : "Use a namespace name: lowercase letters, numbers and hyphens, up to 63 characters."}>
          <TextInput value={namespace} onValueChange={setNamespace} placeholder="All namespaces" invalid={!validNamespace} />
        </Field>
        <Button variant="secondary" disabled={!validNamespace || namespace === savedNamespace}
          onClick={() => { setNamespaceDefault(namespace); setSavedNamespace(namespace); }}>Save namespace</Button>
      </Panel>
      {isTauri() && <Panel title="Request timeout" description="Allow more time for large or distant clusters to respond.">
        <Field label="Request timeout (seconds)" hint={`Currently ${savedTimeout}s. Choose ${REQUEST_TIMEOUT.MIN}–${REQUEST_TIMEOUT.MAX} seconds.`}>
          <TextInput type="number" value={timeout} onValueChange={setTimeout} disabled={busy} invalid={!valid} />
        </Field>
        <Button variant="secondary" disabled={busy || !valid || seconds === savedTimeout} onClick={() => void saveTimeout()}>
          {busy ? "Saving…" : "Save timeout"}
        </Button>
        {error !== null && <FailureAlert tone="sev" title="Could not save request timeout" error={error} />}
      </Panel>}
    </div>
  );
}

export function ApplicationLogsPane() {
  return <Panel title="Application logs" description="Inspect srelens diagnostics, filter entries and open the desktop log file.">
    <Button variant="secondary" onClick={() => openTab("/applog")}>Open application logs</Button>
  </Panel>;
}
