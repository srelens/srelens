/**
 * srelens design system — the reusable component layer.
 *
 * Import primitives from here anywhere in the app:
 *   import { Button, Panel, Table } from "../ui";
 *
 * Importing the barrel also loads the design-system stylesheet, so consumers
 * never have to import CSS separately. Designed for later extraction into a
 * shared `@srelens/ui` package (consumed by the extension API in Phase 4).
 */
import "./styles.css";

export { tokens } from "./tokens";
export type { Tokens } from "./tokens";
export { Button } from "./Button";
export type { ButtonProps, ButtonVariant } from "./Button";
export { ColumnPicker } from "./ColumnPicker";
export type { ColumnOption } from "./ColumnPicker";
export { useColumnVisibility } from "./useColumnVisibility";
export type { ColumnVisibility } from "./useColumnVisibility";
export { TextInput } from "./TextInput";
export type { TextInputProps } from "./TextInput";
export { Field } from "./Field";
export type { FieldProps } from "./Field";
export { Panel } from "./Panel";
export type { PanelProps } from "./Panel";
export { Badge } from "./Badge";
export type { BadgeProps, BadgeVariant } from "./Badge";
export { Spinner } from "./Spinner";
export type { SpinnerProps } from "./Spinner";
export { LoadingState } from "./LoadingState";
export type { LoadingStateProps } from "./LoadingState";
export { Table, filterTableData } from "./Table";
export type { Column, TableProps } from "./Table";
export { Select } from "./Select";
export type { SelectProps, SelectOption } from "./Select";
export { Combobox } from "./Combobox";
export type { ComboboxProps, ComboboxOption } from "./Combobox";
export { Tabs } from "./Tabs";
export type { TabsProps, TabItem } from "./Tabs";
export { ConfirmDialog } from "./ConfirmDialog";
export type { ConfirmDialogProps } from "./ConfirmDialog";
export { StatusPill } from "./StatusPill";
export type { StatusPillProps, StatusKind } from "./StatusPill";
export { IconButton } from "./IconButton";
export type { IconButtonProps } from "./IconButton";
export { KubectlPreview } from "./KubectlPreview";
export type { KubectlPreviewProps } from "./KubectlPreview";
export { Sparkline } from "./Sparkline";
export type { SparklineProps } from "./Sparkline";
export { avatarColor, avatarInitials } from "./avatar";
export { Drawer } from "./Drawer";
export type { DrawerProps } from "./Drawer";
export {
  EmptyState,
  ErrorState,
  DashboardCard,
  DashboardChip,
  DashboardHero,
  DashboardMeter,
  DashboardPage,
  DashboardSegmentBar,
  MetricTile,
  PageHeader,
  PageShell,
  SectionPanel,
  StatusMeter,
  Toolbar,
} from "./Dashboard";
export type { DashboardTone } from "./Dashboard";
// CodeEditor (CodeMirror) is intentionally NOT re-exported here: a static
// barrel re-export would pull it into the main bundle and defeat the lazy
// split. Import it directly from "../ui/CodeEditor" (YamlView does, lazily).
export { getInitialTheme, applyTheme, resolvedThemeMode, THEME_OPTIONS } from "./theme";
export type { Theme, ThemeMode, ThemeName, ThemeOption } from "./theme";
