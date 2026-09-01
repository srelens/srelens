import { notify } from "./notify";

/**
 * Copy a kubectl command to the clipboard.
 *
 * Returns whether it worked, and that return value is the point: a denied or
 * absent clipboard used to be swallowed here without a word, so a copy that
 * never happened was indistinguishable from one that did. The callers that
 * draw a confirmation need to know which they got — saying "Copied" when
 * nothing was copied is the one outcome worse than saying nothing. (#410)
 *
 * The toast stays for the classic design, whose `<Toaster>` renders it. Under
 * the new design it reaches nobody (#374 item 2), which is why the copy
 * controls there confirm on the button instead and do not rely on this.
 */
export async function copyKubectlCommand(command: string): Promise<boolean> {
  try {
    await navigator.clipboard?.writeText(command);
    notify.success("Copied kubectl command");
    return true;
  } catch {
    return false;
  }
}
