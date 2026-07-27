import { isTauri } from "./platform";
import * as tauri from "./tauriTransport";
import * as web from "./webTransport";

/** A function that invokes a backend capability — injectable for testing. */
export type Invoker = <T>(id: string, input?: unknown) => Promise<T>;

const impl = isTauri() ? tauri : web;

export const invokeCapability = impl.invokeCapability;
export const invokeCommand = impl.invokeCommand;
export const on = impl.on;
export const subscribe = impl.subscribe;
export const relaunchApp = impl.relaunchApp;
export const appVersion = impl.appVersion;
