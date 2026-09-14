import { createContext } from "react";
import type { ExtensionResourceSelection } from "@srelens/core";
/** The host owns tab routing; the shared inspector only names the selected resource. */
export const ExtensionResourceNavigation = createContext<((resource: ExtensionResourceSelection) => void) | undefined>(undefined);
