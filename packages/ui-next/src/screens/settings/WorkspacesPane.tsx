import { useId, useState } from "react";
import { describeError, plural } from "@srelens/core";
import { Button, Combobox, ConfirmDialog, IconButton, TextInput } from "@srelens/ui-kit";
import { useContexts, useContextsError, useContextsStatus } from "../../lib/clusters";
import { useOrderedContexts } from "../../lib/contextOrder";
import { Icons } from "../../lib/icons";
import { getMark, useMark } from "../../lib/marks";
import { PALETTE } from "../../lib/markSymbols";
import { workspaceTabCount, type Workspace } from "../../lib/tabs";
import { removeWorkspace, renameWorkspace, setWorkspaceClusters, setWorkspaceColor, useTabs } from "../../lib/tabsStore";

const COLOURS: ReadonlyArray<{ value: string | undefined; label: string }> = [{ value: undefined, label: "None" }, ...PALETTE];

function WorkspaceRow({ workspace, current, lone, onDelete }: {
  workspace: Workspace; current: boolean; lone: boolean; onDelete: () => void;
}) {
  const contexts = useOrderedContexts(useContexts());
  const status = useContextsStatus();
  const listError = useContextsError();
  // A name being typed, committed on Enter or blur; null when not editing.
  const [draft, setDraft] = useState<string | null>(null);
  const group = useId();
  const nameOf = (id: string) => getMark(id, contexts.find((c) => c.stableId === id)?.name ?? id).name;
  const addable = contexts.filter((c) => !workspace.clusters.includes(c.stableId));
  const commit = () => {
    if (draft !== null) renameWorkspace(workspace.id, draft);
    setDraft(null);
  };
  return <li aria-label={workspace.name} aria-current={current || undefined} className="flex flex-col gap-2 border-b border-rule p-3">
    <div className="flex items-center gap-2">
      <div className="min-w-0 flex-1" onBlur={commit}>
        <TextInput aria-label={`Name of ${workspace.name}`} value={draft ?? workspace.name} onValueChange={setDraft} onEnter={commit} />
      </div>
      {current && <span className="text-[0.6875rem] text-muted">Current</span>}
      <Button variant="danger" aria-label={`Delete ${workspace.name}`} disabled={lone}
        title={lone ? "The last workspace cannot be deleted" : undefined} onClick={onDelete}>Delete</Button>
    </div>
    <div role="radiogroup" aria-label={`Colour of ${workspace.name}`} className="flex flex-wrap items-center gap-1">
      {COLOURS.map((swatch) => {
        const on = workspace.color === swatch.value;
        return <input key={swatch.label} type="radio" name={group} aria-label={swatch.label} checked={on}
          onChange={() => setWorkspaceColor(workspace.id, swatch.value)}
          // The radio is the swatch, as in the kit's CustomizeMark: native focus, arrows and role.
          className="h-[18px] w-[18px] cursor-pointer appearance-none rounded-[5px]"
          style={{
            background: swatch.value ?? "transparent",
            border: swatch.value ? undefined : "1px dashed var(--control-line)",
            boxShadow: on ? "0 0 0 2px var(--surface), 0 0 0 3.5px var(--ink)" : undefined,
          }} />;
      })}
    </div>
    <div className="flex flex-wrap items-center gap-1">
      <ul aria-label={`Clusters in ${workspace.name}`} className="flex flex-wrap items-center gap-1">
        {workspace.clusters.map((id) => <li key={id} className="inline-flex items-center gap-1 rounded border border-rule pl-1.5 text-[0.75rem]">
          {nameOf(id)}
          <IconButton icon={Icons.close} label={`Remove ${nameOf(id)} from ${workspace.name}`}
            onClick={() => setWorkspaceClusters(workspace.id, workspace.clusters.filter((c) => c !== id))} />
        </li>)}
      </ul>
      {workspace.clusters.length === 0 && <span className="text-[0.75rem] text-muted">No clusters</span>}
      {/* Only what is known: until a listing answers, the contexts store is
          empty and "every cluster is in" would be a guess. A partial failure
          still offers what it found. */}
      {status === "loading" ? <span className="text-[0.75rem] text-muted">Loading clusters…</span> : <>
        {status === "failed" && <span className="text-[0.75rem]">Could not list clusters: {describeError(listError).raw}</span>}
        {/* Searched rather than listed: a kubeconfig can hold hundreds of contexts.
            Combobox has no disabled state, so it is drawn only when there is one to add. */}
        {addable.length ? <Combobox ariaLabel={`Add a cluster to ${workspace.name}`} value=""
          placeholder="Add cluster…" searchPlaceholder="Search clusters…"
          options={addable.map((c) => ({ value: c.stableId, label: getMark(c.stableId, c.name).name }))}
          onValueChange={(id) => setWorkspaceClusters(workspace.id, [...workspace.clusters, id])} />
          : status === "loaded" && <span className="text-[0.75rem] text-muted">
            {contexts.length ? "Every cluster is in" : "No contexts in your kubeconfig files"}
          </span>}
      </>}
    </div>
  </li>;
}

/**
 * Every workspace in one place: rename, colour, which clusters it holds, and
 * delete. Reads and writes the tabs store directly, so the title-bar chip, the
 * rail and Home follow each change.
 */
export function WorkspacesPane() {
  const { workspaces, workspace: current } = useTabs();
  // Subscribe once so the cluster chips follow a renamed mark.
  useMark("", "");
  const [deleting, setDeleting] = useState<string | null>(null);
  // Found rather than held, so the counts in the dialog are the live ones.
  const target = workspaces.find((w) => w.id === deleting) ?? null;
  return <div className="flex h-full min-h-0 flex-col">
    <header className="border-b border-rule px-3 py-2">
      <h2 className="text-[0.8125rem] font-semibold">Workspaces</h2>
      <p className="text-[0.75rem] text-muted">Each workspace has its own clusters and tabs. Switch between them from the title bar.</p>
    </header>
    <ul aria-label="Workspaces" className="scroll min-h-0 flex-1">
      {workspaces.map((w) => <WorkspaceRow key={w.id} workspace={w} current={w.id === current.id}
        lone={workspaces.length <= 1} onDelete={() => setDeleting(w.id)} />)}
    </ul>
    {target && <ConfirmDialog title={`Delete ${target.name}?`}
      message={`${target.name} holds ${plural(target.clusters.length, "cluster")} and ${plural(workspaceTabCount(target), "open tab")}. The tabs close; the clusters stay in your kubeconfig.`}
      confirmLabel="Delete workspace" danger
      onConfirm={() => { removeWorkspace(target.id); setDeleting(null); }}
      onCancel={() => setDeleting(null)} />}
  </div>;
}
