/** The srelens design system. Components are added as they are merged in. */
export { ActionBar, type ActionBarAction, type ActionBarProps } from "./ActionBar";
export { AgentMark, type AgentMarkProps } from "./AgentMark";
export { Alert, type AlertProps } from "./Alert";
export { ArcField, type ArcFieldProps } from "./ArcField";
export { AskChip, type AskChipProps } from "./AskChip";
export { Avatar, type AvatarProps } from "./Avatar";
export { Badge, type BadgeTone } from "./Badge";
export { Breadcrumb, type BreadcrumbProps } from "./Breadcrumb";
export { Button, type ButtonProps, type ButtonSize, type ButtonVariant } from "./Button";
export { Checkbox, type CheckboxProps } from "./Checkbox";
export { ClusterRail, type ClusterRailItem, type ClusterRailMarker, type ClusterRailProps } from "./ClusterRail";
export { CodeEditor, documentDiagnostics, yamlDiagnostics, type CodeEditorProps } from "./CodeEditor";
export { ColumnPicker, type ColumnOption, type ColumnPickerProps } from "./ColumnPicker";
export { Combobox, type ComboboxOption, type ComboboxProps } from "./Combobox";
export { ConfirmDialog, type ConfirmDialogProps } from "./ConfirmDialog";
export { ConsoleDock, type ConsoleDockProps } from "./ConsoleDock";
export { ConsolePrompt, type ConsolePromptProps } from "./ConsolePrompt";
export { CopyButton, type CopyButtonProps } from "./CopyButton";
export { ContextMenu, type ContextMenuItem, type ContextMenuProps } from "./ContextMenu";
export { CopyAnnounce } from "./CopyAnnounce";
export { CopyCommand, type CopyCommandProps } from "./CopyCommand";
export { CopyIconButton, type CopyIconButtonProps } from "./CopyIconButton";
export { CustomizeMark, type CustomizeMarkProps, type MarkAppearance } from "./CustomizeMark";
export { Dialog, type DialogProps } from "./Dialog";
export { DiffLines, type DiffLinesProps, type DiffRow } from "./DiffLines";
export { Drawer, type DrawerProps } from "./Drawer";
export { DrillCard, type DrillCardProps, type DrillStep } from "./DrillCard";
export { EmptyState, type EmptyStateProps } from "./EmptyState";
export { ErrorState, type ErrorStateProps } from "./ErrorState";
export { Eyebrow, type EyebrowProps } from "./Eyebrow";
export { Field, type FieldProps } from "./Field";
export { FilterBar, type FilterBarProps } from "./FilterBar";
export { IconButton, type IconButtonProps, type IconComponent } from "./IconButton";
export { Inspector, type InspectorFact, type InspectorProps } from "./Inspector";
export { KubectlPreview, type KubectlPreviewProps } from "./KubectlPreview";
export { KV, KVList, type KVListProps, type KVProps } from "./KV";
export { LiveSignal, type LiveSignalProps } from "./LiveSignal";
export { LoadingState, type LoadingStateProps } from "./LoadingState";
export { LogLine, type LogLineProps } from "./LogLine";
export { computeLogWindow, type LogWindow } from "./logWindow";
export { Mark, type MarkProps, type MarkSize } from "./Mark";
export { Meter } from "./Meter";
export { MetricTile, type MetricTileProps } from "./MetricTile";
export { MultiSelect, type MultiSelectProps } from "./MultiSelect";
export { NavIcon, type NavIconProps } from "./NavIcon";
export { PairList, type PairListProps } from "./PairList";
export { Panel, type PanelProps } from "./Panel";
export { OptionCheck } from "./OptionCheck";
export { Popover, type PopoverProps } from "./Popover";
export {
  PortalScopeProvider,
  useOpenLayer,
  usePortalContainer,
  usePortalHost,
  usePortalScoped,
  usePortalShowing,
  type PortalScope,
} from "./portal";
export { Progress, type ProgressProps } from "./Progress";
export { Radio, type RadioProps } from "./Radio";
export { RawError, type RawErrorProps } from "./RawError";
export { ResizeHandle, type ResizeEdge, type ResizeHandleProps } from "./ResizeHandle";
export { type ResourceNode, ResourceTree, type ResourceTreeProps, filterResourceNodes, useFolds } from "./ResourceTree";
export { Screen, type ScreenProps } from "./Screen";
export { Section, type SectionProps } from "./Section";
export { SegmentBar, type SegmentBarProps } from "./SegmentBar";
export { Select, type SelectOption, type SelectProps } from "./Select";
export { Sidebar, type SidebarProps } from "./Sidebar";
export { SideRail, type SideRailProps } from "./SideRail";
export { Sparkline } from "./Sparkline";
export { Spinner, type SpinnerProps } from "./Spinner";
export { Stat, type StatProps } from "./Stat";
export { StatusBar, type StatusBarProps, type StatusSegment } from "./StatusBar";
export { StatusPill, statusTone, type StatusKind, type StatusPillProps } from "./StatusPill";
export { StatusRow, type StatusRowProps } from "./StatusRow";
export { SubHead, type SubHeadProps, type SubHeadVariant } from "./SubHead";
export { SurfaceToast, type SurfaceToastProps } from "./SurfaceToast";
export { Switch, type SwitchProps } from "./Switch";
export { Table, computeVisibleRange, filterTableData, nextSort, type Column, type TableProps, type TableSelection, type TableSort } from "./Table";
export { Tabs, type TabItem, type TabsProps, type TabsVariant } from "./Tabs";
export { TabStrip, type StripTab, type TabStripProps } from "./TabStrip";
export { TextInput, type TextInputProps } from "./TextInput";
export { Titlebar, type TitlebarProps } from "./Titlebar";
export { Toast, type ToastProps } from "./Toast";
export { Toolbar, type ToolbarProps } from "./Toolbar";
export { Tooltip, type TooltipProps } from "./Tooltip";
export { WorkspaceSwitcher, type WorkspaceSummary, type WorkspaceSwitcherProps } from "./WorkspaceSwitcher";
export { type ClusterLink, type WorkspaceCluster, WorkspaceTree, type WorkspaceTreeProps } from "./WorkspaceTree";
export { cx } from "./cx";
export { loadTone, toneColor, toneWash, type Tone } from "./tone";
// `COPIED_MS` stays unexported: the gallery test reads every capitalised name
// in this barrel as a component, and a constant is not one. Nothing outside
// the kit needs the number — the controls that use it live here.
export { useCopied, type CopyState } from "./useCopied";
