import {
  ExtensionManager as Manager,
  ExtensionResourceSlot as ResourceSlot,
  ExtensionControlsProvider,
} from "@srelens/ui-next/extensions";
import { Button, Combobox, Tabs } from "../ui";
import type { ComponentProps } from "react";

// Keep each design's controls and stylesheet; only the extension behavior is shared.
const controls = { Button, Combobox, Tabs };
export function ExtensionManager(props: ComponentProps<typeof Manager>) {
  return (
    <ExtensionControlsProvider value={controls}>
      <Manager {...props} />
    </ExtensionControlsProvider>
  );
}
export function ExtensionResourceSlot(
  props: ComponentProps<typeof ResourceSlot>,
) {
  return (
    <ExtensionControlsProvider value={controls}>
      <ResourceSlot {...props} />
    </ExtensionControlsProvider>
  );
}
