import { useId, useRef, useState, type PointerEvent } from "react";
import { GripVertical } from "lucide-react";
import { deleteContext, describeError, listContexts, type ClusterContext } from "@srelens/core";
import { Button, ConfirmDialog, CustomizeMark, Field, Mark, TextInput } from "@srelens/ui-kit";
import { getKubeconfigFiles, setContexts, useContexts, useContextsError, useContextsStatus } from "../../lib/clusters";
import { moveContext, moveContextBy, useOrderedContexts } from "../../lib/contextOrder";
import { getMark, resetMark, setMark, useMark } from "../../lib/marks";
import { PALETTE, SYMBOLS, symbolFor } from "../../lib/markSymbols";
import { openTab } from "../../lib/tabsStore";

function ContextEditor({ context, onRemove }: { context: ClusterContext; onRemove: () => void }) {
  const mark = useMark(context.stableId, context.name);
  return <section aria-label="Context details" className="scroll min-h-0 min-w-0 flex-1">
    <div className="pane-head">Context appearance</div>
    <CustomizeMark value={mark} onChange={next => setMark(context.stableId, next)}
      onReset={() => resetMark(context.stableId)} colors={PALETTE} icons={SYMBOLS}
      maxImageBytes={64 * 1024} maxNameLength={Infinity} />
    {mark.mark === "image" && <div className="px-3 pb-3"><Field label="Image URL" hint="HTTPS URL or an uploaded image.">
      <TextInput value={mark.imageSrc ?? ""} onValueChange={imageSrc => setMark(context.stableId, { ...mark, imageSrc })} />
    </Field></div>}
    <div className="pane-head border-t border-rule">Connection details</div>
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 p-3 text-[0.75rem]">
      {[["Context", context.name], ["Cluster", context.cluster], ["API server", context.server],
        ["Kubeconfig", context.sourceFile], ["Authentication", context.authKind],
        ["Namespace", context.namespace || "default"]].map(([label, value]) => <div key={label} className="contents">
        <dt className="text-muted">{label}</dt><dd className="overflow-x-auto whitespace-nowrap font-mono" title={value}>{value}</dd>
      </div>)}
    </dl>
    <div className="border-t border-rule p-3"><Button variant="danger" onClick={onRemove}>Remove context</Button></div>
  </section>;
}

export function ClustersPane() {
  const contexts = useOrderedContexts(useContexts());
  const reorderHint = useId();
  const status = useContextsStatus();
  const listingError = useContextsError();
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState<ClusterContext | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [dragging, setDragging] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const drag = useRef<{ source: string; target: string | null } | null>(null);
  const list = useRef<HTMLUListElement>(null);

  // Pointer capture works in the desktop WebView, whose host intercepts
  // native HTML drag/drop. Save only on release; cancellation keeps the order.
  function beginDrag(event: PointerEvent<HTMLButtonElement>, source: string) {
    if (event.button > 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    drag.current = { source, target: null };
    setDragging(source);
  }
  function trackDrag(event: PointerEvent<HTMLButtonElement>) {
    if (!drag.current || !list.current) return;
    const bounds = list.current.getBoundingClientRect();
    if (event.clientY < bounds.top + 24) list.current.scrollTop -= 12;
    else if (event.clientY > bounds.bottom - 24) list.current.scrollTop += 12;
    const row = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>("[data-context]");
    const target = row && list.current.contains(row) ? row.dataset.context ?? null : null;
    drag.current.target = target;
    setDropTarget(target);
  }
  function finishDrag(event: PointerEvent<HTMLButtonElement>, cancel = false) {
    const pending = drag.current;
    drag.current = null;
    setDragging(null); setDropTarget(null);
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (cancel || !pending?.target || pending.target === pending.source) return;
    const sourceIndex = contexts.findIndex(c => c.name === pending.source);
    const targetIndex = contexts.findIndex(c => c.name === pending.target);
    const before = sourceIndex < targetIndex ? contexts[targetIndex + 1]?.name ?? null : pending.target;
    moveContext(contexts, pending.source, before);
  }
  // Subscribe once for the list, including its filter and accessible names.
  useMark("", "");
  const current = contexts.find(c => c.stableId === selected) ?? contexts[0];
  const visible = contexts.filter(c => {
    const mark = getMark(c.stableId, c.name);
    return [c.name, c.server, mark.name, mark.short].join(" ").toLowerCase().includes(query.toLowerCase());
  });
  async function reload() {
    setBusy(true);
    try {
      const outcome = await listContexts(getKubeconfigFiles());
      setContexts(outcome.contexts ?? [], outcome.error ?? "");
    } finally { setBusy(false); }
  }
  async function remove() {
    if (!pending || busy) return;
    setBusy(true); setError("");
    try {
      const result = await deleteContext(pending.name);
      if (!result.success) throw new Error("The context was not removed.");
      resetMark(pending.stableId);
      setContexts(contexts.filter(c => c.stableId !== pending.stableId), listingError);
      setPending(null);
      const outcome = await listContexts(getKubeconfigFiles());
      setContexts(outcome.contexts ?? [], outcome.error ?? "");
    } catch (cause) { setError(describeError(cause).raw); setPending(null); }
    finally { setBusy(false); }
  }
  return <div className="context-management flex h-full min-h-0 flex-col">
    <header className="flex flex-wrap items-center justify-between gap-2 border-b border-rule px-3 py-2">
      <div><h2 className="text-[0.8125rem] font-semibold">Contexts</h2><p className="text-[0.75rem] text-muted">Names, appearance and order are shared with the classic design.</p></div>
      <Button variant="secondary" onClick={() => openTab("/connections")}>Open Connections</Button>
    </header>
    {error && <p role="alert" className="border-b border-rule p-3 text-[0.75rem]">Could not remove context: {error}</p>}
    {status === "failed" && <div role="alert" className="border-b border-rule p-3 text-[0.75rem]">
      Could not list contexts: {describeError(listingError).raw} <Button disabled={busy} onClick={() => void reload()}>Retry</Button>
    </div>}
    {status === "loading" ? <p role="status" className="p-3 text-muted">Loading contexts…</p> : contexts.length === 0 ?
      status !== "failed" && <p className="p-3 text-[0.75rem] text-muted">No contexts configured. Add a kubeconfig or cluster in Connections.</p> :
      <div className="context-management-layout min-h-0 flex-1">
        <section aria-label="Context order" className="flex min-h-0 min-w-0 flex-col border-r border-rule">
          <div className="border-b border-rule p-3"><TextInput aria-label="Filter contexts" placeholder="Filter contexts…" value={query} onValueChange={setQuery} />
            <p id={reorderHint} className="mt-2 text-[0.6875rem] text-muted">Drag to reorder. Focus a handle and use ↑/↓ with a keyboard.</p></div>
          <ul ref={list} className="scroll min-h-0 flex-1" aria-label="Contexts" onDragStart={event => event.preventDefault()}>{visible.map(context => {
            const mark = getMark(context.stableId, context.name);
            const index = contexts.indexOf(context);
            const dropping = dropTarget === context.name && dragging !== context.name;
            const dropAfter = contexts.findIndex(c => c.name === dragging) < index;
            return <li key={context.stableId} data-context={context.name}
              className="context-management-row flex min-w-0 items-center gap-1 border-b border-rule px-2 py-1"
              style={{ background: current?.stableId === context.stableId ? "var(--accent-wash)" : undefined,
                opacity: dragging === context.name ? 0.5 : undefined,
                boxShadow: dropping ? `inset 0 ${dropAfter ? "-2px" : "2px"} var(--accent)` : undefined }}>
              <button type="button" aria-label={`Drag ${mark.name} to reorder`} aria-describedby={reorderHint} title="Drag to reorder"
                className="flex h-6 w-3 shrink-0 cursor-grab touch-none items-center justify-center text-muted active:cursor-grabbing"
                onKeyDown={event => {
                  if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
                  event.preventDefault();
                  const handle = event.currentTarget;
                  moveContextBy(contexts, context.name, event.key === "ArrowUp" ? -1 : 1);
                  requestAnimationFrame(() => { if (handle.isConnected) { handle.focus(); handle.scrollIntoView?.({ block: "nearest" }); } });
                }}
                onPointerDown={event => beginDrag(event, context.name)} onPointerMove={trackDrag}
                onPointerUp={event => finishDrag(event)} onPointerCancel={event => finishDrag(event, true)}
                onLostPointerCapture={event => finishDrag(event, true)}>
                <GripVertical size={12} aria-hidden="true" />
              </button>
              <button type="button" className="flex min-w-0 flex-1 items-center gap-2 text-left" aria-label={`Edit ${mark.name}`} aria-pressed={current?.stableId === context.stableId} onClick={() => setSelected(context.stableId)}>
                <Mark decorative name={mark.name} short={mark.short} color={mark.color} size="sm" withBadge={mark.withText}
                  icon={mark.mark === "icon" ? symbolFor(mark.icon) : undefined} imageSrc={mark.mark === "image" ? mark.imageSrc : undefined} />
                <span className="min-w-0 truncate text-[0.75rem]" title={mark.name === context.name ? context.name : `${mark.name} · ${context.name}`}>{mark.name}</span>
              </button>
            </li>;
          })}</ul>
          {visible.length === 0 && <p className="p-3 text-[0.75rem] text-muted">No matching contexts.</p>}
        </section>
        {current && <ContextEditor key={current.stableId} context={current} onRemove={() => { setError(""); setPending(current); }} />}
      </div>}
    {pending && <ConfirmDialog title="Remove context?" message={`Remove ${pending.name} from ${pending.sourceFile}? This changes your kubeconfig file. The cluster itself is kept.`}
      confirmLabel="Remove context" danger busy={busy} onConfirm={() => void remove()} onCancel={() => setPending(null)} />}
  </div>;
}
