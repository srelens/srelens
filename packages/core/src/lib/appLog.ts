import { invokeCommand } from "../transport/transport";

/** Read the tail of srelens's own application log file. */
export async function readAppLog(maxBytes?: number): Promise<string> {
  return (await invokeCommand<string>("read_app_log", { maxBytes: maxBytes ?? null })) ?? "";
}

/** The absolute path of the application log file (for display / copy). */
export async function appLogPath(): Promise<string> {
  return (await invokeCommand<string>("app_log_path")) ?? "";
}

/** Reveal the log file in the OS file manager. */
export async function revealAppLog(): Promise<void> {
  await invokeCommand("reveal_app_log");
}
