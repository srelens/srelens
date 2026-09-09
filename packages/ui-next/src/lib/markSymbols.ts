import { Boxes, Cloud, Globe2, Shield } from "lucide-react";
import type { IconComponent } from "@srelens/ui-kit";
import { Icons } from "./icons";

/**
 * The colours a cluster may be marked with.
 *
 * Tokens rather than hex, so a mark set in the dark theme is not a colour that
 * only worked there. Each is named too: {@link CustomizeMark} reads the label
 * aloud, and "#b4342a" names nothing. The custom picker beside them is still
 * there for anyone who wants a twelfth.
 *
 * The `--mark-*` axis rather than the five semantic tokens this used to reuse
 * (`--accent`, `--ok`, `--info`, `--warn`, `--sev`). Those say something — this
 * is bad, this needs attention — and `--accent` moves with the accent axis, so
 * a cluster marked violet turned blue for anyone who preferred a blue accent.
 * A mark's colour is identity, not meaning, and identity should not move.
 */
export const PALETTE = [
  { value: "var(--mark-red)", label: "Red" },
  { value: "var(--mark-orange)", label: "Orange" },
  { value: "var(--mark-amber)", label: "Amber" },
  { value: "var(--mark-green)", label: "Green" },
  { value: "var(--mark-teal)", label: "Teal" },
  { value: "var(--mark-blue)", label: "Blue" },
  { value: "var(--mark-indigo)", label: "Indigo" },
  { value: "var(--mark-purple)", label: "Purple" },
  { value: "var(--mark-pink)", label: "Pink" },
  { value: "var(--mark-slate)", label: "Slate" },
  { value: "var(--mark-ink)", label: "Ink" },
];

/**
 * The symbols a mark may be drawn as, in place of its initials.
 *
 * The kit ships no icon set, so the catalogue is the app's — and it is the
 * whole of what `mark: "icon"` can mean: an id stored here that is not in this
 * list draws nothing, and {@link Mark} falls back to the initials underneath
 * rather than to an empty coloured square.
 *
 * Named after the picture rather than after what the glyph means elsewhere in
 * the app: this is someone choosing a badge for a cluster, and "Workloads" is
 * not a thing anybody is picking. The ids are stored, so they are stable
 * whatever the pictures behind them become.
 */
export const SYMBOLS: Array<{ id: string; label: string; icon: IconComponent }> = [
  { id: "cluster", label: "Cluster", icon: Boxes },
  { id: "cloud", label: "Cloud", icon: Cloud },
  { id: "globe", label: "Globe", icon: Globe2 },
  { id: "server", label: "Server", icon: Icons.cluster },
  { id: "layers", label: "Layers", icon: Icons.workloads },
  { id: "box", label: "Box", icon: Icons.pods },
  { id: "database", label: "Database", icon: Icons.statefulsets },
  { id: "disk", label: "Disk", icon: Icons.storage },
  { id: "network", label: "Network", icon: Icons.network },
  { id: "shield", label: "Shield", icon: Shield },
  { id: "key", label: "Key", icon: Icons.secrets },
  { id: "terminal", label: "Terminal", icon: Icons.terminal },
  { id: "compass", label: "Compass", icon: Icons.investigate },
  { id: "wheel", label: "Ship's wheel", icon: Icons.helmreleases },
  { id: "wrench", label: "Wrench", icon: Icons.toolbox },
];

export const symbolFor = (id: string | undefined): IconComponent | undefined =>
  SYMBOLS.find((symbol) => symbol.id === id)?.icon;
