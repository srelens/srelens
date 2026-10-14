import { useCallback, useState, type ReactNode } from "react";
import { TextInput } from "@srelens/ui-kit";
import { useProtection, type Protection } from "./protection";

/**
 * The typed confirmation a protected cluster asks for (#101): the cluster's
 * name, as the rail shows it, typed into the dialog that is about to change it.
 *
 * One hook for every dialog that asks. `useClusterGate` composes it for the
 * confirms that already share that gate; Helm, the shell pickers, app actions
 * and agent consent call it directly. A second copy of this check is the one
 * that would drift: a different trim, a looser compare, a different sentence.
 *
 * `contextName` is whatever `protectionFor` reads: a context's name, or the
 * pinned ID an app surface or a sidecar's request names it by.
 *
 * The text is trimmed — a paste leaves spaces — and otherwise compared
 * exactly, case included.
 */
export interface ProtectedConfirm {
  protection: Protection | null;
  /** The line and the field, for inside a dialog. `null` when not protected. */
  field: ReactNode;
  /** `Type <name> to confirm.` until the name matches; `null` then, or when not protected. */
  refusal: string | null;
  /** `refusal === null`. */
  ready: boolean;
  /** Clears what was typed. The same function on every render. */
  reset: () => void;
}

export function useProtectedConfirm(contextName: string | null | undefined): ProtectedConfirm {
  const protection = useProtection(contextName);
  const [typed, setTyped] = useState("");
  const reset = useCallback(() => setTyped(""), []);

  const refusal =
    protection && typed.trim() !== protection.name ? `Type ${protection.name} to confirm.` : null;

  return {
    protection,
    field: protection ? (
      <div className="my-2 flex flex-col gap-1" data-slot="protected-confirm">
        <p className="m-0">{`${protection.name} is protected (workspace ${protection.workspace}).`}</p>
        <TextInput value={typed} onValueChange={setTyped} aria-label={`Type ${protection.name} to confirm`} />
      </div>
    ) : null,
    refusal,
    ready: refusal === null,
    reset,
  };
}
