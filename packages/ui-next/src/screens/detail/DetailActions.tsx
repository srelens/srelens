import { ActionBar, Button, type ActionBarAction, type ContextMenuItem } from "@srelens/ui-kit";
import { useAsk } from "../../console";
import { Icons } from "../../lib/icons";
import { askQuestion } from "../../lib/kinds/rowAffordances";
import { ROW_ACTION_LABEL } from "../../lib/kinds/rowActions";
import type { KindActions, ListRow } from "../../lib/kinds/types";
import { useRowMenu } from "../ResourceMenu";

/**
 * How many WORDED actions stay on the peek's row before the rest fold into
 * the overflow. Every action with an icon is drawn as one and always stays:
 * that is classic's quick tools row, and a built-in kind's actions all have
 * icons (#1072). An app's actions have none, so they keep their words, and
 * two words fit beside `Ask` in a pane 260px wide at its narrowest.
 */
const ON_BAR = 2;

/**
 * The same count for the full tab's header row, which the design draws with
 * four — `Logs`, `Shell`, `Debug`, `Edit` beside `Ask` and the overflow.
 *
 * The actions, their order, their icons, their confirms and their kubectl
 * previews are the row menu's in both hosts. What changes between the peek's
 * quick tools and a window-wide header is how they are drawn and how many
 * stay out of the overflow.
 */
const ON_HEADER_BAR = 4;

/**
 * The shorter words the design puts on the bar, for the three entries whose
 * menu labels carry a verb the bar has no room for.
 *
 * Only these three, and only on the bar: the menu is a menu and can afford
 * "Follow logs". Keyed through {@link ROW_ACTION_LABEL} rather than spelt out,
 * so renaming an entry in the row menu is a type error here rather than a
 * silent reversion to the long label.
 */
const BAR_LABEL: Record<string, string> = {
  [ROW_ACTION_LABEL.logs]: "Logs",
  [ROW_ACTION_LABEL.shell]: "Shell",
  [ROW_ACTION_LABEL.forward]: "Forward",
};

/**
 * The row menu's entries, as bar actions.
 *
 * `ContextMenuItem` and `ActionBarAction` describe the same thing in two
 * vocabularies — `onPick`/`onSelect`, and an `id` the menu does not need
 * because a menu row is identified by what it says. So the label IS the id,
 * which is sound because `useRowMenu` never emits the same label twice in one
 * list. Separators go: a bar has no rows to divide, and `ActionBar` already
 * draws the destructive ones in the danger tone from `danger` alone.
 *
 * "Open in new tab" is the one entry dropped. In the peek the header already
 * carries "Open tab" (the design draws it there, outlined, beside the close),
 * and in the full tab host the pane IS the tab — an entry that opens a second
 * copy of what you are looking at.
 */
function toBarActions(items: ContextMenuItem[]): ActionBarAction[] {
  const actions: ActionBarAction[] = [];
  for (const item of items) {
    if (item.kind === "sep" || item.label === ROW_ACTION_LABEL.openTab) continue;
    actions.push({
      id: item.label,
      label: BAR_LABEL[item.label] ?? item.label,
      icon: item.icon,
      danger: item.danger,
      confirmLabel: item.confirmLabel,
      onSelect: item.onPick,
    });
  }
  return actions;
}

export interface DetailActionsProps {
  context: string;
  kind: string;
  /** The API group, for a custom kind — Edit's route carries it (see `editRoute`). */
  group?: string;
  namespace: string | null;
  name: string;
  /**
   * Which actions this kind offers, off its `KindDescriptor` — the same table
   * the list's row menu reads. The design's per-kind middle pair falls out of
   * it; nothing here branches on a kind's name.
   */
  actions: KindActions;
  /**
   * The subject's own health verdict, which decides which question `Ask`
   * sends. From `resourceStatusLine`, the same read the header's dot uses.
   */
  flagged: boolean;
  /** CronJob only: `spec.suspend`, so Suspend/Resume is labelled the right way
   *  round. `useRowMenu` reads it off the row, as it does for a list. */
  suspended?: boolean;
  /** Node only: `spec.unschedulable`, so Cordon/Uncordon is labelled the right
   *  way round, for the same reason and by the same route. */
  unschedulable?: boolean;
  /** A write made from this bar went through: read the subject again, so
   *  the two fields above are the object's as it now is. */
  onChanged?: () => void;
  /**
   * Which host is drawing the row. The peek draws every action as a quick
   * tool under the name with a compact `Ask` last; the full tab's header puts
   * `Ask` first and has room for four worded actions.
   *
   * The two hosts are no longer one pane (R-5 is retired), but their actions
   * still are — so what varies is a layout, named here once, rather than a
   * second list of the same verbs somewhere else.
   */
  host?: "peek" | "tab";
}

export interface DetailActionRowProps {
  /** The cluster `Ask` is about. */
  context: string;
  /**
   * What `Ask`'s question calls the resource. A built-in's name is enough, since
   * `askContextFor` reads the rest off its route. An app's route says nothing
   * there, so its row names the kind and namespace here.
   */
  subject: string;
  /** The subject's own health verdict, which decides which question `Ask` sends. */
  flagged: boolean;
  actions: ActionBarAction[];
  /** What the group of actions is called: "Pod actions". */
  label: string;
  host?: "peek" | "tab";
}

/**
 * The row itself. In the peek: the actions as quick tools, icons where they
 * have one, then a compact `Ask` at the end (#1072). In the tab's header:
 * `Ask`, then four worded actions, then the overflow. An app's resource draws
 * this with its own reviewed actions; a built-in one through
 * {@link DetailActions}.
 */
export function DetailActionRow({ context, subject, flagged, actions, label, host = "peek" }: DetailActionRowProps) {
  const ask = useAsk(context);
  const question = askQuestion(subject, flagged);
  const Sparkle = Icons.ask;
  const inPeek = host === "peek";

  // The visible word is short and the same on every subject, so the question
  // goes in the accessible name, where it says what will actually be sent —
  // the same split `AskChip` makes. In the peek it is one more tool, sized
  // like them and pushed to the end, its sparkle carrying the accent the
  // filled button used to.
  const askButton = ask && (
    <Button
      type="button"
      size="sm"
      variant={inPeek ? "secondary" : undefined}
      className={inPeek ? "ml-auto" : undefined}
      aria-label={`Ask: ${question}`}
      title={`Ask: ${question}`}
      onClick={() => ask(question)}
    >
      <Sparkle size={12} aria-hidden="true" className={inPeek ? "text-accent" : undefined} />
      Ask
    </Button>
  );

  if (!inPeek) {
    return (
      <>
        {askButton}
        <ActionBar actions={actions} label={label} max={ON_HEADER_BAR} />
      </>
    );
  }
  return (
    <>
      <ActionBar actions={actions} label={label} iconOnly max={actions.filter((a) => a.icon).length + ON_BAR} />
      {askButton}
    </>
  );
}

/**
 * The design's action row: `Ask` and the kind's own actions.
 *
 * Drawn under the name in the peek, as quick tools, and along the top of the
 * full tab — a placement the host chooses, not two sets of actions. `host`
 * says which layout, and nothing else about this row varies between them.
 *
 * Every action in it is the list's row menu, unchanged — `useRowMenu` already
 * owns Follow logs, Open shell, Port forward, Edit, Copy as kubectl,
 * Suspend/Resume, Run now, Scale, Restart rollout, Evict and Delete, along
 * with the confirm each destructive one takes and the kubectl preview inside
 * it. A second implementation of that list is the thing this file exists to
 * avoid: it renders the hook's `dialog` as well as its items, so a Delete in
 * the overflow reaches the very confirm a Delete in the row menu does.
 *
 * Two adaptations, and only two. The hook is typed on `ListRow` — the shape a
 * table hands it — while this pane holds a `K8sObject`, so the row is
 * reconstituted from the identity the pane already has. And it answers in
 * `ContextMenuItem`s, which {@link toBarActions} projects onto `ActionBar`'s
 * vocabulary.
 *
 * `Ask` is a `Button` rather than the row's `AskChip`: the chip is
 * `opacity: 0` until its row is hovered, which is right for one of forty rows
 * and wrong for the only control on a bar. The question itself is the chip's,
 * through {@link askQuestion} — one phrasing per gesture, wherever it is
 * drawn.
 */
export function DetailActions({
  context,
  kind,
  namespace,
  name,
  actions,
  flagged,
  suspended,
  unschedulable,
  onChanged,
  host = "peek",
  group,
}: DetailActionsProps) {
  const { items, dialog } = useRowMenu({ context, kind, actions, group, onChanged });

  // What a table would have handed the hook. `suspended` is not on `ListRow`
  // — `useRowMenu` reads it off the row with the same cast a `CronJobSummary`
  // needs — so it rides along the same way, and so does a Node's
  // `unschedulable`.
  const row: ListRow & { suspended?: boolean; unschedulable?: boolean } = {
    name,
    ...(namespace === null ? {} : { namespace }),
    ...(suspended === undefined ? {} : { suspended }),
    ...(unschedulable === undefined ? {} : { unschedulable }),
  };
  return (
    <>
      <DetailActionRow
        context={context}
        subject={name}
        flagged={flagged}
        actions={toBarActions(items(row))}
        label={`${kind} actions`}
        host={host}
      />
      {dialog}
    </>
  );
}
