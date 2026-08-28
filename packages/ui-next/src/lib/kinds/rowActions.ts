/**
 * Every word the row menu puts on a row, in one place.
 *
 * `ContextMenuItem` has no id — the kit describes a menu row, and a menu row
 * is identified by what it says. So the label IS the identity, and a second
 * surface built from the same list (the resource detail pane's footer bar,
 * which drops one entry and shortens three others for a bar a few hundred
 * pixels wide) has to be able to name the entries it means. Naming them
 * through these constants is what turns a rename into a type error rather than
 * a footer that silently goes back to being a generic list — which is a change
 * no test could see coming if both sides spelt the strings out.
 *
 * A leaf module rather than an export of `screens/ResourceMenu`, and this is
 * load-bearing: `ResourceMenu` reaches `lib/tabsStore` → `lib/tabs` →
 * `lib/routes` → `screens/Resources` → the detail pane, which imports the
 * footer, which needs these words. Read off `ResourceMenu` itself they arrive
 * `undefined` at module-evaluation time, halfway round that circle. Here there
 * is no circle to be halfway round.
 *
 * The values are the menu's, and the menu is where they are still rendered.
 */
export const ROW_ACTION_LABEL = {
  openTab: "Open in new tab",
  logs: "Follow logs",
  shell: "Open shell",
  forward: "Port forward",
  edit: "Edit",
  copy: "Copy as kubectl",
  suspend: "Suspend",
  resume: "Resume",
  trigger: "Run now",
  scale: "Scale",
  restart: "Restart rollout",
  evict: "Evict",
  delete: "Delete",
} as const;
