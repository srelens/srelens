import {
  ArrowLeftRight,
  Ban,
  BellRing,
  Bot,
  Box,
  BriefcaseBusiness,
  Circle,
  Check,
  Clock3,
  Compass,
  Copy,
  Cpu,
  Database,
  ExternalLink,
  FileCog,
  FilePlus,
  FolderOpen,
  FolderTree,
  Gauge,
  GitBranch,
  HardDrive,
  KeyRound,
  Layers,
  LayoutDashboard,
  LayoutGrid,
  Lock,
  ListOrdered,
  LogOut,
  Maximize,
  Moon,
  Network,
  Pause,
  Pencil,
  Play,
  Plus,
  Puzzle,
  RadioTower,
  RefreshCw,
  RotateCcw,
  RotateCw,
  Route,
  ScrollText,
  Search,
  Server,
  ServerCog,
  Settings2,
  Shield,
  ShieldCheck,
  ShipWheel,
  Signpost,
  Sparkles,
  SlidersHorizontal,
  Scaling,
  Sun,
  Terminal,
  TimerReset,
  Trash2,
  TriangleAlert,
  UserRoundCheck,
  UserRoundCog,
  Waves,
  Waypoints,
  Webhook,
  Wrench,
  X,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import type { IconComponent } from "@srelens/ui-kit";

/**
 * The shell's glyphs, in one place.
 *
 * Two reasons it is a map rather than an import at each call site. The kit
 * takes no dependency on an icon set — it describes the hole an icon goes in
 * with `IconComponent` and leaves the choosing to the app — so the choosing has
 * to happen somewhere in ui-next, and spread across a dozen components it
 * becomes a dozen places to look when the same concept is drawn two different
 * ways. And the names here are the app's vocabulary, not lucide's: `workloads`
 * and `access` say what the sidebar group is, while `Layers` and `ShieldCheck`
 * say what the picture is, and the sidebar should not have to know the second
 * to ask for the first.
 *
 * Typed as the kit's `IconComponent` rather than lucide's `LucideIcon`, so
 * everything that consumes this map is typed against the hole and not against
 * the icon set filling it — swapping sets, or hand-drawing one glyph as an
 * inline SVG, is then a change to this file alone.
 *
 * The resource glyphs follow the classic app's (`apps/desktop/src/ui/NavIcon`)
 * deliberately: the same kind should not be a box in one design and a cube in
 * the other for someone who has both installed during the migration.
 */
export const Icons = {
  // The titlebar and the window's own actions.
  sun: Sun,
  moon: Moon,
  zoomIn: ZoomIn,
  zoomOut: ZoomOut,
  zoomReset: Maximize,
  settings: Settings2,
  workspace: LayoutGrid,
  // §25's lock surface. Nothing in `@srelens/ui-kit` carries a padlock — the
  // kit takes no dependency on an icon set at all, which is why every glyph in
  // this design comes from here — and the shell had none either: the lock
  // screen was drawing its own inline SVG. One entry, so the mark and any
  // later lock affordance draw the same padlock.
  lock: Lock,
  add: Plus,
  close: X,
  search: Search,
  warn: TriangleAlert,

  // The sidebar's groups. One per group in the resource tree.
  cluster: Server,
  workloads: Layers,
  config: FileCog,
  network: Network,
  storage: HardDrive,
  access: ShieldCheck,
  crds: Puzzle,
  investigate: Compass,

  // Cluster.
  overview: LayoutDashboard,
  nodes: Server,
  namespaces: FolderTree,
  events: BellRing,

  // Workloads.
  pods: Box,
  deployments: Layers,
  statefulsets: Database,
  daemonsets: ServerCog,
  replicasets: Copy,
  jobs: BriefcaseBusiness,
  cronjobs: Clock3,

  // Config.
  configmaps: FileCog,
  secrets: KeyRound,
  resourcequotas: Gauge,
  limitranges: SlidersHorizontal,
  horizontalpodautoscalers: Scaling,
  poddisruptionbudgets: ShieldCheck,
  priorityclasses: ListOrdered,
  runtimeclasses: Cpu,
  leases: TimerReset,
  mutatingwebhookconfigurations: Webhook,
  validatingwebhookconfigurations: Webhook,

  // Network.
  services: Network,
  endpoints: RadioTower,
  endpointslices: GitBranch,
  ingresses: Route,
  ingressclasses: Signpost,
  networkpolicies: Shield,
  portforwards: ArrowLeftRight,

  // Storage.
  persistentvolumeclaims: HardDrive,
  persistentvolumes: Database,
  storageclasses: Layers,

  // Access control.
  serviceaccounts: UserRoundCog,
  clusterroles: Shield,
  roles: ShieldCheck,
  clusterrolebindings: UserRoundCheck,
  rolebindings: UserRoundCheck,

  // Investigate, and the rest of the app's surfaces.
  control: LayoutDashboard,
  incidents: BellRing,
  topology: Waypoints,
  agent: Bot,
  logs: ScrollText,
  terminal: Terminal,
  helmreleases: ShipWheel,
  toolbox: Wrench,
  newresource: FilePlus,

  /** Promote what is on screen into a tab of its own — the detail peek's
   *  "Open tab". */
  openTab: ExternalLink,

  /** Hand what is on screen to the agent — the row's ask chip wears its own
   *  inline glyph (the kit takes no icon-set dependency), so this is for the
   *  detail pane's footer button, which is a `Button` like any other. */
  ask: Sparkles,

  // The application log screen's own actions.
  refresh: RefreshCw,
  copy: Copy,
  check: Check,
  reveal: FolderOpen,

  // The row menu's write actions, shared by every kind that offers them.
  edit: Pencil,
  /** Put something back the way it was — the mark editor's Reset. */
  revert: RotateCcw,
  scale: Scaling,
  restart: RotateCw,
  evict: LogOut,
  /** Stop new pods being scheduled to a node — the design's crossed circle. */
  cordon: Ban,
  /** Move every pod off a node — the design's wave. */
  drain: Waves,
  trash: Trash2,
  pause: Pause,
  play: Play,

  /** What a kind with no glyph of its own gets — a CRD, most often. */
  fallback: Circle,
} as const satisfies Record<string, IconComponent>;

/** Every glyph this map knows, for a caller looking one up by name. */
export type IconName = keyof typeof Icons;
