import { createContext } from "react";
import {
  Button as KitButton,
  Combobox as KitCombobox,
  Tabs as KitTabs,
} from "@srelens/ui-kit";
export const ExtensionControls = createContext({
  Button: KitButton,
  Combobox: KitCombobox,
  Tabs: KitTabs,
});
export const ExtensionControlsProvider = ExtensionControls.Provider;
