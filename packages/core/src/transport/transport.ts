import { isTauri } from "./platform";
import * as tauri from "./tauriTransport";
import * as web from "./webTransport";

/** A function that invokes a backend capability — injectable for testing. */
export type Invoker = <T>(id: string, input?: unknown) => Promise<T>;

const impl = isTauri() ? tauri : web;

// A reloaded desktop window still holds the streams its last page opened; end
// them as this page loads (#700), not only when it first opens one. The web
// host has no such command (#727).
if (isTauri()) void tauri.resetWindowStreams();

export const invokeCapability = impl.invokeCapability;
export const invokeCommand = impl.invokeCommand;
export const on = impl.on;
export const subscribe = impl.subscribe;
export const relaunchApp = impl.relaunchApp;
export const appVersion = impl.appVersion;
export const setWebviewZoom = impl.setWebviewZoom;
export const onWindowCloseRequested = impl.onWindowCloseRequested;
export const currentWindowLabel: () => string = () =>
  isTauri() ? tauri.currentWindowLabel() : web.currentWindowLabel();

