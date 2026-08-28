import React, { useState } from "react";
import { ConfirmDialog, TextInput, Button } from "../ui";
import { startPortForward } from "@srelens/core";
import { saveForward } from "@srelens/core";

function validPort(v: string): number | null {
  const n = Number(v);
  return Number.isInteger(n) && n >= 1 && n <= 65535 ? n : null;
}

/** Extract the backend-suggested free port from a local-port conflict error,
 *  e.g. "port 8080 is already in use; 54321 is free" (see `BindError::InUse`
 *  in crates/kube/src/forward.rs). Returns null for any other error. */
function suggestedPortFrom(message: string): number | null {
  const match = /(\d+)\s+is free\b/.exec(message);
  return match ? Number(match[1]) : null;
}

/**
 * Prompt for a remote (and optional local) port, then start a port-forward to
 * a Pod or Service. Leaving the local port blank lets the OS pick a free one.
 */
export function ForwardDialog({
  context,
  namespace,
  kind,
  name,
  defaultRemotePort,
  onClose,
}: {
  context: string;
  namespace: string;
  kind: string;
  name: string;
  defaultRemotePort?: number;
  onClose: () => void;
}) {
  const [remote, setRemote] = useState(defaultRemotePort ? String(defaultRemotePort) : "");
  const [local, setLocal] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [suggestedPort, setSuggestedPort] = useState<number | null>(null);
  const [saveState, setSaveState] = useState<"idle" | "busy" | "saved">("idle");

  /** Start the forward. `overrideLocalPort` (used by "use it" on a conflict
   *  suggestion) wins over whatever is in the local-port field. */
  async function start(overrideLocalPort?: number) {
    const remotePort = validPort(remote);
    if (remotePort === null) {
      setError("Enter a remote port between 1 and 65535");
      return;
    }
    let localPort: number | undefined = overrideLocalPort;
    if (localPort === undefined && local.trim()) {
      const l = validPort(local);
      if (l === null) {
        setError("Local port must be between 1 and 65535");
        return;
      }
      localPort = l;
    }
    setBusy(true);
    setError("");
    setSuggestedPort(null);
    try {
      await startPortForward({ context, namespace, kind, name, remotePort, localPort });
      onClose();
    } catch (e) {
      const message = String(e);
      setError(message);
      setSuggestedPort(suggestedPortFrom(message));
      setBusy(false);
    }
  }

  /** Retry the forward on the backend's suggested free local port. */
  async function useSuggestedPort() {
    if (suggestedPort === null) return;
    setLocal(String(suggestedPort));
    await start(suggestedPort);
  }

  /** Persist the current target as a reusable shortcut (doesn't start it). */
  async function handleSave() {
    const remotePort = validPort(remote);
    if (remotePort === null) {
      setError("Enter a remote port between 1 and 65535");
      return;
    }
    let localPort: number | undefined;
    if (local.trim()) {
      const l = validPort(local);
      if (l === null) {
        setError("Local port must be between 1 and 65535");
        return;
      }
      localPort = l;
    }
    setSaveState("busy");
    try {
      await saveForward(context, {
        id: crypto.randomUUID(),
        name,
        namespace,
        kind,
        target: name,
        remotePort,
        localPort,
      });
      setSaveState("saved");
    } catch (e) {
      setError(String(e));
      setSaveState("idle");
    }
  }

  return (
    <ConfirmDialog
      title={`Forward ${kind.toLowerCase()} port`}
      message={
        <div className="flex flex-col gap-3">
          <p className="m-0 text-sm text-muted-foreground">
            Forward a local port to <code>{name}</code>
            {namespace ? (
              <>
                {" "}
                in <code>{namespace}</code>
              </>
            ) : null}
            .
          </p>
          <div className="flex gap-3">
            <label className="flex flex-col gap-1 text-xs text-muted-foreground">
              {kind === "Service" ? "Service port" : "Container port"}
              <div className="w-28">
                <TextInput
                  value={remote}
                  onValueChange={setRemote}
                  placeholder="e.g. 80"
                  aria-label="Remote port"
                />
              </div>
            </label>
            <label className="flex flex-col gap-1 text-xs text-muted-foreground">
              Local port (optional)
              <div className="w-28">
                <TextInput
                  value={local}
                  onValueChange={setLocal}
                  placeholder="auto"
                  aria-label="Local port"
                />
              </div>
            </label>
          </div>
          <div>
            <Button
              variant="secondary"
              disabled={saveState !== "idle"}
              onClick={() => void handleSave()}
            >
              {saveState === "saved" ? "Saved" : "Save this forward"}
            </Button>
          </div>
          {error && (
            <div className="flex flex-col items-start gap-2">
              <p className="m-0 text-sm text-destructive">Error: {error}</p>
              {suggestedPort !== null && (
                <Button variant="secondary" disabled={busy} onClick={() => void useSuggestedPort()}>
                  Use port {suggestedPort}
                </Button>
              )}
            </div>
          )}
        </div>
      }
      confirmLabel="Forward"
      busy={busy}
      onConfirm={() => void start()}
      onCancel={onClose}
    />
  );
}
