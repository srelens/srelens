import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ConsoleDock, Eyebrow, cx } from "@srelens/ui-kit";
import { useConsole } from "../console";
import { askAgent, clearAgentRun, useAgentRun } from "../lib/agentRun";
import {
  commandsFor,
  matchCommands,
  type Command,
  type CommandDeps,
  type CommandGroup,
} from "../lib/agentCommands";
import { suggestionsFor } from "../lib/agentSuggestions";
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

/** Suggestions mode: the eyebrow `Start here` over three route-aware prompts,
 *  each of which asks immediately when picked — a suggestion is already a
 *  full question, not a draft to edit first. */
function SuggestionList({ items, onAsk }: { items: readonly string[]; onAsk: (question: string) => void }) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <Eyebrow>Start here</Eyebrow>
      {items.map((question) => (
        <button
          key={question}
          type="button"
          className="min-w-0 truncate rounded-tile px-2 py-1.5 text-left text-sm hover:bg-sunk"
          onClick={() => onAsk(question)}
        >
          {question}
        </button>
      ))}
    </div>
  );
}

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
 * verbatim, when nothing matches); an empty run is `suggestionsFor` the
 * active route; anything else is `<Transcript compact />` over the shared
 * store `lib/agentRun.ts` holds. The full `/agent` screen is a second
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
export function Console({ apple, onToggleTheme }: { apple: boolean; onToggleTheme: () => void }) {
  const sealed = useWorkspaceSealed();
  const { open, setOpen, scope, registerSubmit } = useConsole();
  const [value, setValue] = useState("");
  const { turns, gates, busy } = useAgentRun();
  const contexts = useContexts();
  const activeCtx = useActiveContext();
  const { tabs, activeId, workspaces } = useTabs();
  const route = tabs.find((t) => t.id === activeId)?.route ?? "/";
  const context = activeCtx?.name ?? "";

  const deps = useMemo<CommandDeps>(
    () => ({
      route,
      context,
      clusters: contexts.map((c) => ({ id: c.stableId, name: c.name })),
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
    [route, context, contexts, workspaces, onToggleTheme],
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
    void askAgent(raw);
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
  } else if (turns.length === 0) {
    children = (
      <SuggestionList
        items={suggestionsFor(route)}
        onAsk={(question) => {
          void askAgent(question);
          setValue("");
        }}
      />
    );
  } else {
    // `live={false}`: `ConsoleDock`'s own body already declares its own ARIA
    // log region, polite or off, around whichever children it is given
    // (`dockLive` below) — a second one nested inside it would announce
    // inconsistently and often twice (I7).
    children = <Transcript compact live={false} turns={turns} gates={gates} />;
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
      onClear={() => clearAgentRun()}
      live={dockLive}
    >
      {children}
    </ConsoleDock>
  );
}
