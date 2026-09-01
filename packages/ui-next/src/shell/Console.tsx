import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Alert, ConsoleDock, Eyebrow, cx } from "@srelens/ui-kit";
import { useConsole } from "../console";
import {
  askAgent,
  clearAgentRun,
  dismissAgentError,
  chooseAgent,
  getRunSubject,
  selectRun,
  stopAgentRun,
  useActiveRunKey,
  useRun,
} from "../lib/agentRun";
import {
  commandsFor,
  matchCommands,
  type Command,
  type CommandDeps,
  type CommandGroup,
} from "../lib/agentCommands";
import { AgentPicker } from "../screens/agent/AgentPicker";
import { LOADING, type Read } from "../lib/read";
import { askContextFor, runKeyFor } from "../lib/askContext";
import { useNamespaces } from "../lib/workspace";
import { readImageFile } from "../lib/pastedImages";
import { isTauri, listAgents, type AgentInfo } from "@srelens/core";
import { useActiveContext, useContexts } from "../lib/clusters";
import { detailRoute } from "../lib/detailRoute";
import { hint } from "../lib/shortcuts";
import { openTab, setActiveCluster, switchWorkspace, useTabs } from "../lib/tabsStore";
import { logsRoute } from "../screens/Logs";
import { Transcript } from "../screens/agent/Transcript";
import { useWorkspaceSealed } from "./LockGate";

/** §F's four palette groups, in the order the mock lists them. */
const GROUPS: readonly CommandGroup[] = ["Action", "Go", "Cluster", "Workspace"];

/** §F's empty-palette line, verbatim. */
const NO_COMMAND_MATCH = "No command matches. Press ⏎ to ask the agent instead.";

/** Command palette mode: the matched commands, grouped under §F's four
 *  headings — a group with nothing in it draws no heading at all, which is
 *  what lets `Action`/`Go` vanish on a route with no resource for free. */
function CommandRows({ commands, onRun }: { commands: readonly Command[]; onRun: (c: Command) => void }) {
  return (
    <div className="flex min-w-0 flex-col gap-2">
      {GROUPS.map((group) => {
        const rows = commands.filter((c) => c.group === group);
        if (rows.length === 0) return null;
        return (
          <div key={group} className="flex min-w-0 flex-col gap-1">
            <Eyebrow>{group}</Eyebrow>
            {rows.map((c) => (
              <button
                key={c.id}
                type="button"
                className="flex min-w-0 items-center justify-between gap-2 rounded-tile px-2 py-1.5 text-left text-sm hover:bg-sunk"
                onClick={() => onRun(c)}
              >
                <span className={cx("min-w-0 truncate", c.danger && "text-sev")}>{c.label}</span>
                <span className="min-w-0 shrink-0 truncate text-xs text-faint">{c.hint}</span>
              </button>
            ))}
          </div>
        );
      })}
    </div>
  );
}

/**
 * The console docked along the bottom of the window: §F's three transcript
 * modes over the one agent run every tab shares, joined to the kit's
 * `ConsoleDock` and the provider's `open`/`scope`/`registerSubmit`.
 *
 * **Three modes, picked by what there is to show, not by a mode flag this
 * component remembers.** The query starting with `/` is the command palette
 * (matched against `commandsFor`'s own list, or §F's empty-palette line,
 * verbatim, when nothing matches); a conversation with something in it is
 * `<Transcript compact />` over the shared store `lib/agentRun.ts` holds; and
 * anything else — an empty run, or the full view where the screen draws the
 * conversation itself — is NOTHING, so the dock is just its composer. The full `/agent` screen is a second
 * renderer over the SAME store — not built here, and not this component's to
 * mount (`Composer`, Task 5's own submit surface, belongs to that screen; this
 * dock keeps `ConsoleDock`'s own single-line prompt, wired straight to
 * `askAgent`, which is what the collapsed strip's `Ask about <context>` is).
 *
 * **The workspace cover is a render-time guard here, not only a mount-time
 * one.** In the shipped tree `LockGate` unmounts this component along with
 * the whole band when the vault seals, which already stops it — but this
 * component is also mounted on its own, with no `LockGate` above it, by
 * anything that renders `<Console>` in isolation (this file's own tests
 * included). `useWorkspaceSealed()` is what keeps "hidden while the vault is
 * locked" true in both trees rather than only the one with a gate over it —
 * see §1 of the mock and `AgentConsent`'s own refusal under the same cover.
 *
 * **`CommandDeps.context` is the real cluster name, never the dock's own
 * `scope` label.** `scope` (`prod-eu / checkout-api`) is what the accent pill
 * shows; every command that reaches core — `openAction`, `openResource`, a
 * `Cluster` switch — needs the bare kubeconfig context name instead, which is
 * `useActiveContext()`'s own answer and nothing this component derives from
 * the label.
 */
export function Console({ fullView }: { fullView?: boolean }) {
  const sealed = useWorkspaceSealed();
  // From context, not props: this mounts in two places now — the window's
  // bottom edge on most screens, and the foot of `/agent`'s own main column so
  // that screen's rail can be a full-height sibling.
  const {
    open,
    setOpen,
    scope,
    registerSubmit,
    apple,
    onToggleTheme,
    // The draft lives in the provider, not here: this component has two mount
    // points — the window's bottom edge and `/agent`'s own main column — and
    // switching between them unmounts one and mounts the other, which silently
    // lost whatever was typed and any screenshot pasted with it.
    draft: value,
    setDraft: setValue,
    images,
    setImages,
  } = useConsole();
  /**
   * Screenshots waiting to go with the next question, as data URIs so they can
   * be shown before they are sent.
   *
   * Pasting one was simply never possible in the new design — not a
   * regression, a gap: the deleted `Composer` had no image handling either.
   */
  const [attachError, setAttachError] = useState<unknown>(null);
  /** A question refused because srelens has no cluster to ask about. */
  const [noCluster, setNoCluster] = useState(false);
  /** How many pasted or picked images are still being read. */
  const [reading, setReading] = useState(0);
  /**
   * Which agent the next question goes to — read here because the picker lives
   * in the composer's footer now, beside `+`.
   *
   * It was in `/agent`'s rail, which put it on one screen only; the composer is
   * on every screen, and choosing the agent is part of asking.
   */
  const [agents, setAgents] = useState<Read<AgentInfo[]>>(LOADING);

  useEffect(() => {
    let cancelled = false;
    listAgents()
      .then((v) => {
        if (!cancelled) setAgents({ kind: "ready", value: v });
      })
      .catch((e: unknown) => {
        if (!cancelled) setAgents({ kind: "error", error: e });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // `available && !gated` filtered before the picker sees the list: an agent
  // that is installed but gated must not be offered, and filtering inside the
  // picker risks a call site that forgets.
  const offered = agents.kind === "ready" ? agents.value.filter((a) => a.available && !a.gated) : [];

  async function attach(files: File[]) {
    setAttachError(null);
    // Counted, because `FileReader` is asynchronous and Enter is not. Submit
    // before a read settles and the question went WITHOUT the image, cleared
    // the attachment row, and then this callback appended the image — silently
    // carrying it onto the next question instead. `onSubmit` refuses while
    // this is above zero.
    setReading((n) => n + 1);
    try {
      const uris = await Promise.all(files.map(readImageFile));
      setImages((held) => [...held, ...uris]);
    } catch (e) {
      // Said, not swallowed: a screenshot the reader believes is attached and
      // is not would be discovered only by the answer ignoring it.
      setAttachError(e);
    } finally {
      setReading((n) => n - 1);
    }
  }
  const contexts = useContexts();
  const activeCtx = useActiveContext();
  const { tabs, activeId, workspace, workspaces } = useTabs();
  const route = tabs.find((t) => t.id === activeId)?.route ?? "/";
  /**
   * Whether this dock IS the full view's own composer — a transcript directly
   * above it, and a screen that already says how to begin.
   *
   * Two sources, deliberately OR-ed. `fullView` is the structural fact, passed
   * by the one mount site that knows it (`screens/Agent.tsx`); `route` is the
   * same fact read back out of the tab store. They can disagree only if the
   * mount and the store disagree about which screen is showing — and in that
   * case suppressing is right on either's word, since what is being suppressed
   * is a "Start here" under a finished answer and a link to the screen the
   * reader is already on.
   *
   * Route alone was the guard, and "Start here" was reported back under a full
   * transcript anyway.
   */
  const isFullView = fullView === true || route === "/agent";
  const context = activeCtx?.name ?? "";
  // The reader's standing namespace narrowing for THIS cluster — the picker on
  // the list screens. Without it, a question asked from a list narrowed to one
  // namespace had the agent sweep every namespace in the cluster.
  // What a question asked from here is ABOUT. Derived from the active route,
  // which is where a resource's identity lives — a cluster name alone left the
  // agent with no target for "summarise this stream" and it went searching
  // four namespaces for one.
  // The reader's standing namespace narrowing for THIS cluster — the picker on
  // the list screens. Without it, a question asked from a list narrowed to one
  // namespace had the agent sweep every namespace in the cluster.
  const selected = useNamespaces(activeCtx?.stableId);
  const about = useMemo(() => askContextFor(route, context, selected), [route, context, selected]);
  /**
   * The dock shows the conversation about the thing the reader is LOOKING at,
   * and follows them as they navigate — it is not a window onto whichever run
   * was asked into last.
   *
   * That is the whole point of scoping runs by subject: the dock on a pod's
   * logs is the conversation about that pod, and walking to a StatefulSets
   * list puts the dock on that list's conversation instead. `/agent` is the
   * surface that stays put, on whatever the rail selected.
   *
   * A key with no run yet reads as the empty run, which is right — the reader
   * has navigated somewhere they have not asked about.
   */
  /**
   * `/agent` is not a subject — it is the FULL VIEW of whichever conversation
   * is selected — so there the dock shows the active run rather than one keyed
   * by its own route.
   *
   * Without this the two surfaces on that screen were on different
   * conversations: the transcript showed the subject the reader had asked
   * about, while the dock, keyed on `/agent`, showed an empty run and offered
   * "Start here" suggestions underneath a full answer. Reported from use.
   */
  const activeKey = useActiveRunKey();
  const runKey = useMemo(
    () => (route === "/agent" ? activeKey : runKeyFor(about, route)),
    [route, activeKey, about],
  );
  const { turns, gates, busy, error, agentKind } = useRun(runKey);

  const deps = useMemo<CommandDeps>(
    () => ({
      route,
      context,
      // Only the clusters THIS workspace holds. `setActiveCluster` refuses an
      // id outside `workspace.clusters` and returns the workspace untouched
      // (`tabsStore.ts:426`), but the command went on to `openTab` regardless —
      // so picking a kubeconfig context the workspace does not include
      // relabelled a tab with that cluster's name while the active cluster
      // never moved, and every action taken under that tab then ran against
      // the previous cluster. A tab that names one cluster and acts on another
      // is the #380 class of defect, seven findings deep.
      //
      // Filtered here rather than refused in the command: a command that
      // cannot do what its label says should not be offered at all, which is
      // the same rule §F's absent `Action` group already follows.
      clusters: contexts
        .filter((c) => workspace.clusters.includes(c.stableId))
        .map((c) => ({ id: c.stableId, name: c.name })),
      workspaces,
      openTab,
      // `setActiveCluster` itself, unwrapped: its own signature already takes
      // `(id, clusterName?)`, which is a superset of what `CommandDeps` asks
      // for — passing both, always, is `agentCommands.ts`'s own contract, not
      // something this wiring has to enforce a second time.
      setActiveCluster,
      switchWorkspace,
      onToggleTheme,
      openAction: (a) => {
        // Ruling C: navigate to the resource, where its own row menu and
        // detail actions already own the confirm for `restart`/`scale` — not
        // a second one raised over the dock. This lands the reader on the
        // resource's tab; it does not also reopen the SPECIFIC confirm the
        // command named, which would need a way to carry that intent across
        // a tab open that nothing here builds (see `openResource` below for
        // the same limit on `shell`/`forward`).
        openTab(detailRoute(a.kind, a.namespace || null, a.name), { clusterName: a.context });
      },
      openResource: (r) => {
        if (r.as === "logs") {
          // The one shape with a route of its own: `logsRoute` bakes
          // kind/namespace/name into the URL itself, so this is the one `Go`
          // command whose label ("Follow logs · <name>") is a promise the
          // navigation actually keeps.
          openTab(logsRoute(r.kind, r.namespace, r.name), { clusterName: r.context });
          return;
        }
        // `shell` and `forward` have no such route — see `agentCommands.ts`'s
        // module doc for why, and why their labels no longer name the
        // resource. `r.kind`/`r.namespace`/`r.name` go unused here on
        // purpose: the day `NewSessionMenu` grows a `pod` field, or a target
        // rides along a route param into `/forwards`, this is where that
        // identity is already sitting, fully specified — do not delete the
        // unused fields as dead weight, and do not re-add the resource's name
        // to the command's label until the navigation can actually reach it.
        openTab(r.as === "shell" ? "/terminals" : "/forwards", { clusterName: r.context });
      },
    }),
    [route, context, contexts, workspace, workspaces, onToggleTheme],
  );

  const commands = useMemo(() => commandsFor(deps), [deps]);

  function onSubmit(raw: string) {
    if (raw.startsWith("/")) {
      const matched = matchCommands(commands, raw.slice(1));
      if (matched.length > 0) {
        matched[0].run();
        setValue("");
        return;
      }
      // §F's own words for this: no command matched, so what was typed is
      // asked as a question instead of being discarded.
    }
    /*
      In a browser this component renders `null` further down — nothing served
      by `srelens server` can answer a question, since `api_command.rs` has no
      `chat_*` arm. That guard is BELOW the effect that registers this handler,
      so a screen-level Ask button (`Overview.tsx`'s, for one) still reached
      `askAgent`, whose `chat_start` came back unsupported, and the dock that
      would have shown the failure was not on screen to show it.

      So a question asked in web mode does the one useful thing available: it
      opens `/agent`, which carries the explanation of why the agent needs the
      desktop app. Handled here rather than by skipping the registration — an
      Ask button that silently does nothing is the same dead end by a quieter
      route.
    */
    if (!isTauri()) {
      openTab("/agent", { clusterName: context || undefined });
      return;
    }
    // #7: every MCP tool call takes an explicit context, so a question sent
    // with no cluster lets the agent pick one — and the run is keyed under an
    // empty cluster, so it vanishes from the dock the moment a context does
    // resolve. Refused where the reader can see it, rather than sent and left
    // to fail somewhere they cannot.
    if (context === "") {
      setNoCluster(true);
      return;
    }
    setNoCluster(false);
    // An image still being read belongs to THIS question. Sending now would
    // send the question without it and attach it to the next one.
    if (reading > 0) return;
    /*
      In the full view the dock shows whichever conversation is SELECTED, and
      `/agent` is not that conversation's subject — it is not a subject at all.
      Submitting with this component's own route recomputed the destination as
      `<cluster>|/agent`, so a follow-up typed under a pod's transcript started
      a separate run and left that conversation's CLI resume behind.

      The conversation remembers what it is about (`RunState.subject`), so the
      follow-up goes back to it. A run restored from a file written before that
      was recorded has none, and falls back to this route — the old behaviour.
    */
    const selected = isFullView ? getRunSubject(runKey) : undefined;
    void askAgent(raw, {
      // The KEY, not a route to re-derive one from. `runKey` is what this dock
      // is showing, and in the full view it cannot always be reconstructed: a
      // conversation opened beside a live one about the same subject is
      // aliased, and a dock expanded before its first question has no stored
      // subject at all. Both re-derivations reached a different run.
      ...(isFullView && runKey !== null ? { key: runKey } : {}),
      // The subject still supplies the preface, so a follow-up carries the
      // resource the conversation is about rather than the cluster alone.
      about: selected?.about ?? about,
      route: selected?.route ?? route,
      images: images.length > 0 ? images : undefined,
    });
    setImages([]);
    setValue("");
    setOpen(true);
  }

  // A ref, not `[registerSubmit]` alone closing over a stale `onSubmit`: a
  // question asked from elsewhere (`ask()`) can arrive long after this
  // effect last ran, and it must reach the CURRENT `commands`/`route`, not
  // whichever ones were in scope when the dock first mounted. The same
  // pattern `Window.tsx`'s own `runRef` uses for its accelerator handler.
  const onSubmitRef = useRef(onSubmit);
  onSubmitRef.current = onSubmit;
  useEffect(() => registerSubmit((q) => onSubmitRef.current(q)), [registerSubmit]);


  // After every hook above, never before: the guard decides what renders,
  // not which hooks run, so the hook order stays identical whether the
  // workspace is covered or not.
  if (sealed) return null;
  // Nothing in a browser can answer a question. `askAgent`'s first backend call
  // is `chat_start`, and `api_command.rs`'s match has no `chat_*` or
  // `agent_list` arm, so every question would 404. A permanently dead prompt
  // fixed to the bottom of every tab is worse than no prompt: the reader is
  // told to ask and then told it failed. `/agent` is where the explanation
  // goes — a reader looking for the agent goes there, and it says so on
  // arrival, which is the same choice Settings makes for the MCP server.
  if (!isTauri()) return null;
  // Renders on `/agent` TOO. That screen has no bar of its own: this dock is
  // the one prompt in the app, on every screen including the agent's own.
  // It used to mount a bespoke `Composer` there, which is what put two input
  // boxes on that screen and made one of them look like a different product.

  const commandMode = value.startsWith("/");
  const exchanges = turns.filter((t) => t.role === "user").length;

  let children: ReactNode;
  if (commandMode) {
    const matched = matchCommands(commands, value.slice(1));
    children =
      matched.length === 0 ? (
        <p className="min-w-0 break-words text-sm text-muted">{NO_COMMAND_MATCH}</p>
      ) : (
        <CommandRows
          commands={matched}
          onRun={(c) => {
            c.run();
            setValue("");
          }}
        />
      );
  } else if (isFullView || turns.length === 0) {
    /*
      Nothing. A dock with nothing in it is a composer, not a panel with a
      placeholder in it.

      This branch used to be "Start here" over three route-aware suggestions.
      They were reported three times running — under a finished answer in the
      full view, then as prompts nobody wanted at all — and asked to go:
      "remove question not needed, make the dock clean". A canned question is
      also the one thing on this surface srelens cannot say is about anything
      the reader is looking at.

      In the full view it is nothing even WITH turns: the screen above draws
      that conversation, and a compact copy of it in the dock is the same
      conversation twice.
    */
    children = undefined;
  } else {
    // `live={false}`: `ConsoleDock`'s own body already declares its own ARIA
    // log region, polite or off, around whichever children it is given
    // (`dockLive` below) — a second one nested inside it would announce
    // inconsistently and often twice (I7).
    children = <Transcript compact live={false} turns={turns} gates={gates} />;
  }

  // Run-level, above whichever of the three modes is showing. The store held
  // this in `error` and nothing drew it, so an `ask()` chip pressed mid-turn
  // was refused in silence — which is what the refusal was supposed to stop.
  if (error !== undefined) {
    children = (
      <div className="flex min-w-0 flex-col gap-2">
        <Alert tone="sev" title="That question was not sent" onDismiss={() => dismissAgentError(runKey)}>
          <p className="m-0">{error}</p>
        </Alert>
        {children}
      </div>
    );
  }

  // The ONE ARIA log region for the dock, on `ConsoleDock`'s own body — `true`
  // only for the thread, which is the console's ordinary content and the one
  // mode `ConsoleDock`'s own doc means by "announce what arrives in the
  // output" (`ConsoleDock.tsx:46-50`). `false` for the palette and the
  // suggestions list: both re-render their entire body on every keystroke
  // that changes the query, and a polite region would read the whole matched
  // list out again on every character (I7's scenario, `/re` re-announcing
  // three times).
  const dockLive = !commandMode && turns.length > 0;

  return (
    <ConsoleDock
      // In the full view the screen carries the transcript, the context and a
      // `New question` of its own; the dock's own header and body would each
      // be a second copy.
      composerOnly={isFullView}
      open={open}
      onOpenChange={setOpen}
      value={value}
      onValueChange={setValue}
      onSubmit={onSubmit}
      busy={busy}
      mode={commandMode ? "Command" : "Agent"}
      // Empty rather than absent is a bordered chip with nothing in it, and
      // the console is unscoped until `Window`'s own effect (`contextLabelFor`)
      // has scoped it to the active route and cluster.
      context={scope || undefined}
      status={exchanges > 0 ? `${exchanges} exchange${exchanges === 1 ? "" : "s"}` : undefined}
      placeholder={scope ? `Ask about ${scope}` : "Ask about this cluster"}
      shortcutHint={hint("console", apple)}
      onClear={() => clearAgentRun(runKey)}
      // The dock and `/agent` are two views of ONE conversation, and until now
      // there was no way between them: a reader had to know the left nav has an
      // Agent entry under Investigate. Reported as "how to go to full mode".
      //
      // Selects this dock's own run first, so the screen opens on the
      // conversation the reader was just looking at rather than on whichever
      // was asked into last. That is the one thing that could be surprising
      // here, and it is the reason this is not simply `openTab("/agent")`.
      // Absent in the full view itself: a control that opens the screen the
      // reader is already looking at does nothing they can see.
      onExpand={
        isFullView
          ? undefined
          : () => {
              selectRun(runKey);
              openTab("/agent", { clusterName: context || undefined });
            }
      }
      // The only Stop in the app: the agent screen's own composer is gone.
      promptLead={
        offered.length > 0 ? (
          <AgentPicker
            agents={offered}
            selectedKind={agentKind}
            // The run this picker is SHOWING. The dock is keyed by its own
            // route, which off `/agent` need not be the active run — so
            // without this, picking the agent a restored conversation is not
            // on could return early as a no-op.
            onSelect={(kind) => chooseAgent(kind, runKey)}
            disabled={busy}
          />
        ) : undefined
      }
      onStop={busy ? () => stopAgentRun() : undefined}
      onPasteImages={(files) => void attach(files)}
      onPickImages={(files) => void attach(files)}
      promptContext={
        // What the question is about, said where the question is typed. The
        // header already carries a context pill, but the header belongs to the
        // dock's transcript; this belongs to the composer.
        // ONLY what nothing else on screen says. The scope was a chip here as
        // well as the dock header's pill AND the prompt's own placeholder —
        // the same long context name three times in a column, reported as
        // "this still has duplicate in chat box and header, drop from one
        // place". It is dropped from here, where the placeholder directly
        // beneath it already reads `Ask about <scope>`.
        //
        // The namespace stays: it is the one part of the scope no other line
        // states, and it is what the question will actually be narrowed to.
        about.namespaces?.length === 1 ? (
          <span className="chip">
            <span>{about.namespaces[0]}</span>
          </span>
        ) : undefined
      }
      attachments={
        <>
          {images.map((uri, i) => (
            <span key={i} className="chip">
              {/* The image itself, not a filename: a pasted screenshot has no
                  name, and a row of identical "image.png" chips says nothing
                  about which is which. */}
              <img src={uri} alt={`Attachment ${i + 1}`} className="h-4 w-4 rounded-sm object-cover" />
              <span>image {i + 1}</span>
              <button
                type="button"
                aria-label={`Remove attachment ${i + 1}`}
                className="text-faint hover:text-ink"
                onClick={() => setImages((held) => held.filter((_, j) => j !== i))}
              >
                ✕
              </button>
            </span>
          ))}
          {attachError !== null && (
            <span className="chip" style={{ color: "var(--sev)" }}>
              <span>That image could not be read</span>
            </span>
          )}
          {noCluster && (
            <span className="chip" style={{ color: "var(--sev)" }}>
              <span>No cluster is active — connect one before asking</span>
            </span>
          )}
          {reading > 0 && (
            <span className="chip">
              <span>Reading {reading === 1 ? "an image" : `${reading} images`}…</span>
            </span>
          )}
        </>
      }
      live={dockLive}
    >
      {children}
    </ConsoleDock>
  );
}
