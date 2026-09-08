import { useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { isTauri } from "@srelens/core";
import { Screen } from "@srelens/ui-kit";
import type { RoutedScreenProps } from "../lib/routes";
import { ApplicationLogsPane, KubernetesPane, WorkspacePane } from "./settings/PreferencePanes";
import { UpdatesPane } from "./settings/UpdatesPane";
import { AgentAccess } from "./settings/AgentAccess";
import { AgentPane } from "./settings/AgentPane";
import { AppearancePane } from "./settings/AppearancePane";
import { AuditPane } from "./settings/AuditPane";
import { McpServer } from "./settings/McpServer";
import { SecurityPane } from "./settings/SecurityPane";
import { AccessibilityPane, ClustersPane, ShortcutsPane } from "./settings/SmallPanes";

/** Settings use the new design's stores and shared core commands. */
/** §23's rail width for this screen, and this screen's alone (§A.1's table). */
const NAV_WIDTH = 196;

type SectionId = "agent" | "security" | "appearance" | "accessibility" | "shortcuts" | "workspace" | "kubernetes" | "logs" | "updates" | "clusters";

/**
 * §23's nav, in §23's order, minus `Deep links`.
 *
 * `desktopOnly` is carried here rather than checked at the render site so the
 * order lives in exactly one list: a second array for the web build is a
 * second thing to keep in step, and the two would drift the first time a
 * section is added.
 *
 * **`Security` is desktop-only because every vault command is a Tauri
 * command.** `vaultLock`, `vaultBiometricStatus`/`Enable`/`Disable` and
 * `vaultChangePassword` (`packages/core/src/lib/mcpSecurity.ts`) all go through
 * `invoke` from `@tauri-apps/api/core` — not through the transport layer that
 * has a web half — so in a browser every one of them rejects before it reaches
 * a server. The pane would render an honest run of failure states over
 * controls that cannot act, which is the shape this project has twice decided
 * against: the toolbox draws no install column on the web and says why once,
 * and Connections offers no file adding there for the same reason. A control
 * that cannot work is not drawn, and the reason is said once — in the rail,
 * where the missing entry would have been.
 *
 * **`Agent & MCP` keeps its entry on the web, and loses two of its four
 * panes.** It is the section this screen OPENS ON, and `McpServer` and
 * `AuditPane` were mounted there unconditionally — but `getMcpToken()`,
 * `mcpHttpStatus()` and `auditTail()` are direct `invoke`s from
 * `@tauri-apps/api/core` (`packages/core/src/lib/mcpSecurity.ts`,
 * `packages/core/src/lib/mcp.ts`) with no web half, so all three rejected on
 * every visit and the default pane was two failure alerts over a server that
 * cannot be started and a trail that cannot be read. Same rule as `Security`,
 * one level down: the panes are not drawn, and the reason is said once — this
 * time inside the section, because what is missing here is a panel and not an
 * entry.
 *
 * The ENTRY stays, because `AgentAccess` is not desktop-only: it reads
 * srelens's own capability registry through `gatedCapabilityIds`, which is
 * plain data compiled into the bundle and equally true in a browser — and it
 * is the pane a web reader most wants, since it is the one that says what a
 * connected agent may do without asking. A section with real content is not
 * removed for the panes it cannot fill; `desktopOnly` is for a section that
 * would be left empty.
 *
 * `AgentPane` stays for the same reason and by the same test, not by
 * assumption: `llmGetSettings`, `llmSetKey`, `llmClearKey`, `llmKeyStatus`,
 * `llmListModels` and `listAgents` (`packages/core/src/lib/llm.ts`,
 * `packages/core/src/lib/chat.ts`) all go through `invokeCommand` — the
 * transport that dispatches to a web implementation, not the bare
 * `@tauri-apps/api/core` `invoke` that fails outright in a browser — so an
 * operator's provider keys and agent CLI inventory are exactly as real on the
 * web as they are on the desktop.
 */
const SECTIONS: ReadonlyArray<{ id: SectionId; label: string; desktopOnly?: true }> = [
  { id: "agent", label: "Agent & MCP" },
  { id: "security", label: "Security", desktopOnly: true },
  { id: "appearance", label: "Appearance" },
  { id: "accessibility", label: "Accessibility" },
  { id: "shortcuts", label: "Shortcuts" },
  { id: "workspace", label: "Workspace" },
  { id: "kubernetes", label: "Kubernetes" },
  { id: "logs", label: "Application logs" },
  { id: "updates", label: "Updates", desktopOnly: true },
  { id: "clusters", label: "Clusters" },
];

/**
 * Nothing of its own. The four fields every routed screen is handed —
 * `route`, `ported`, `onSwitchToClassic` and `onLocked` — are the only props
 * this screen takes, and `onLocked` is the one it is here for:
 * `RoutedScreenProps` declares it, `Body` forwards it, and `LockGate` above the
 * tab strip is what it raises. It used to be declared here as an OPTIONAL prop with a
 * `console.error` stand-in, because the surface did not exist yet; it exists
 * now, so the stand-in is gone and an omitted handler is a typecheck failure at
 * the call site rather than a lock button that reports itself to a console
 * nobody is reading.
 */
export type SettingsProps = RoutedScreenProps;

export function Settings({ ported, onSwitchToClassic, onLocked }: SettingsProps) {
  const desktop = isTauri();
  const visible = SECTIONS.filter((s) => !s.desktopOnly || desktop);

  const [updatesOpened, setUpdatesOpened] = useState(false);
  const [active, setActive] = useState<SectionId>(SECTIONS[0].id);
  const headId = useId();
  const tabBase = useId();
  const tabId = (id: SectionId) => `${tabBase}-${id}`;
  const refs = useRef(new Map<SectionId, HTMLButtonElement>());

  function select(id: SectionId) {
    if (id === "updates") setUpdatesOpened(true);
    setActive(id);
  }

  function focus(id: SectionId) {
    select(id);
    // Selection follows focus, as it does in the kit's own `Tabs`: the panes
    // are already mounted-on-demand and cheap to switch, and this is what tabs
    // whose panels are not expensive are expected to do.
    refs.current.get(id)?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    // Computed from what is FOCUSED, not from `active` — the kit's `Tabs`
    // learned this in #323 review: a parent that has not committed the change
    // yet sent the second arrow key off from a tab the reader had left.
    const focused = visible.findIndex((s) => refs.current.get(s.id) === document.activeElement);
    const index = focused >= 0 ? focused : visible.findIndex((s) => s.id === active);
    if (index < 0) return;
    let next: number | null = null;
    // Down and Up, not Right and Left: the rail is a column, and it says so
    // with `aria-orientation`.
    if (event.key === "ArrowDown") next = (index + 1) % visible.length;
    else if (event.key === "ArrowUp") next = (index - 1 + visible.length) % visible.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = visible.length - 1;
    if (next === null) return;
    // Otherwise Up/Down also scroll the rail and Home/End jump the page.
    event.preventDefault();
    focus(visible[next].id);
  }

  function pane(id: SectionId): ReactNode {
    switch (id) {
      case "agent":
        // §23's section, now over four panes: what the agent may do
        // (`AgentAccess`), the LLM provider keys and CLI inventory it runs on
        // (`AgentPane`), the server it is reached through, and what it has
        // done. The last two are desktop-only — see {@link SECTIONS} for why,
        // and why this section is still drawn on the web without them.
        // `AgentPane` IS one of them, and the comment here used to say
        // otherwise. The reasoning was that `llmGetSettings`, `llmSetKey` and
        // the rest go through `invokeCommand`, "the transport that has a web
        // half" — which is true and beside the point. A transport is not a
        // handler: `webTransport` dutifully POSTs to
        // `/api/command/llm_get_settings`, and `api_command.rs`'s match has no
        // `llm_*` arm, so the server answers `404 unknown command`. The pane
        // opened on four failed reads and no control that could work.
        //
        // `AgentAccess` genuinely does stay: it calls `gatedCapabilityIds` and
        // `isTauri` and no backend command at all.
        return (
          <div className="flex flex-col gap-4">
            <AgentAccess />
            {desktop ? (
              <>
                <AgentPane />
                <McpServer />
                <AuditPane />
              </>
            ) : (
              /* Said ONCE, inside the section that lost them — not in the
                 rail, which reports an absent ENTRY and stays about
                 `Security`. Not an `Alert` either: nothing has failed, and
                 the whole point is that nothing is asked to. */
              <p
                data-testid="no-agent-server"
                className="text-[0.75rem] leading-relaxed text-muted"
              >
                The agent, the MCP server and its audit trail live in the srelens desktop app.
                Provider keys, model lists, the agent CLIs, starting the loopback server and
                reading the audit log are all desktop commands, so there is nothing here for them
                to act on.
              </p>
            )}
          </div>
        );
      case "security":
        return <SecurityPane onLocked={onLocked} />;
      case "appearance":
        return <AppearancePane ported={ported} onSwitchToClassic={onSwitchToClassic} />;
      case "accessibility":
        return <AccessibilityPane />;
      case "shortcuts":
        return <ShortcutsPane />;
      case "workspace":
        return <WorkspacePane />;
      case "kubernetes":
        return <KubernetesPane />;
      case "logs":
        return <ApplicationLogsPane />;
      case "updates":
        return null;
      case "clusters":
        return <ClustersPane />;
    }
  }

  return (
    <Screen title="Settings" eyebrow="workspace" fill>
      <div className="flex min-h-0 flex-1">
        {/* Not `SideRail`: that component puts its rail on the RIGHT, with a
            `border-left` and the main region first, and §A.1's table lists
            Settings among the three screens whose rail is on the left. A `side`
            flag on a component whose whole subject is "a main region beside a
            fixed rail" would be a second layout inside it, and this is a nav
            rather than the "About this kind" material `SideRail` is for.
            A `complementary` landmark all the same, named by its own head, for
            the reason `SideRail` gives: this is material a reader may want to
            jump to and may equally want to skip. */}
        <aside
          aria-labelledby={headId}
          className="flex min-h-0 shrink-0 flex-col border-r border-rule bg-surface"
          style={{ width: NAV_WIDTH }}
        >
          <div id={headId} className="pane-head">
            Settings
          </div>
          <div className="scroll min-h-0 flex-1 p-1.5">
            <div
              role="tablist"
              aria-orientation="vertical"
              aria-label="Settings sections"
              className="flex flex-col gap-0.5"
              onKeyDown={onKeyDown}
            >
              {visible.map((section) => (
                <button
                  key={section.id}
                  id={tabId(section.id)}
                  type="button"
                  role="tab"
                  className="nav-item"
                  data-active={section.id === active}
                  aria-selected={section.id === active}
                  aria-controls={`${tabBase}-panel`}
                  // Roving tabindex: the rail is one Tab stop, and Tab from it
                  // moves past the rail rather than through six sections.
                  tabIndex={section.id === active ? 0 : -1}
                  ref={(node) => {
                    if (node) refs.current.set(section.id, node);
                    else refs.current.delete(section.id);
                  }}
                  onClick={() => select(section.id)}
                >
                  {section.label}
                </button>
              ))}
            </div>
            {!desktop && (
              /* Said ONCE, here, where the entry would have been — the same
                 shape the toolbox uses for its absent install column. Not an
                 `Alert`: this is not a fault and not about the section on
                 screen, it is the footnote to a nav that is one entry shorter
                 than the desktop's. */
              <p
                data-testid="no-security"
                className="mt-3 border-t border-rule px-2 pt-2 text-[0.6875rem] leading-snug text-muted"
              >
                Security lives in the srelens desktop app. The master passphrase, locking and Touch
                ID are all desktop commands, so there is nothing here for them to act on.
              </p>
            )}
          </div>
        </aside>
        {/* `min-w-0` as well as `min-h-0`. A flex item's implicit
            `min-width: auto` refuses to shrink below its content, and the audit
            table is as wide as its widest target — so without it a long
            namespace-qualified name widens this column and pushes the 196px
            rail off the window instead of scrolling inside the pane. That exact
            defect has shipped twice on this migration and jsdom shows none of
            it, which is why the suite asserts the property rather than the
            layout. */}
        <div
          data-slot="settings-content"
          id={`${tabBase}-panel`}
          role="tabpanel"
          aria-labelledby={tabId(active)}
          // The one Tab stop into the pane's own content, so a reader arrowing
          // to a section can Tab straight into it.
          tabIndex={0}
          className={`scroll min-h-0 min-w-0 flex-1${active === "updates" || active === "clusters" ? "" : " p-3"}`}
        >
          {pane(active)}
          {desktop && updatesOpened && (
            <div hidden={active !== "updates"}><UpdatesPane /></div>
          )}
        </div>
      </div>
    </Screen>
  );
}
