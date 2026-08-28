import { notify } from "./notify";

/** Copy a kubectl command to the clipboard, silently ignoring a denied/unavailable clipboard (matching ResourceOverview's copy affordance). */
export async function copyKubectlCommand(command: string): Promise<void> {
  try {
    await navigator.clipboard?.writeText(command);
    notify.success("Copied kubectl command");
  } catch {
    /* clipboard unavailable — ignore, matching ResourceOverview's copy affordance */
  }
}
