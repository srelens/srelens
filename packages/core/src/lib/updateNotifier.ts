import { checkForUpdate, type UpdateMeta } from "./updater";
import type { UpdateChannel } from "./settings";

interface Options {
  /** Override the check (for tests); defaults to the real manifest check. */
  check?: (channel: UpdateChannel) => Promise<UpdateMeta | null>;
  /** Suppress the notification for a version already surfaced to the user. */
  alreadyNotified?: (version: string) => boolean;
}

/**
 * Run an automatic update check and call `onAvailable` when a newer version
 * exists. Unlike the manual check in Settings, this is a background poll: it
 * swallows errors (a failed check must not nag) and skips versions the user has
 * already been told about, so it can run on startup and on a timer.
 */
export async function checkForUpdateAndNotify(
  channel: UpdateChannel,
  onAvailable: (update: UpdateMeta) => void,
  options: Options = {},
): Promise<void> {
  const check = options.check ?? checkForUpdate;
  let update: UpdateMeta | null;
  try {
    update = await check(channel);
  } catch {
    return;
  }
  if (update && !options.alreadyNotified?.(update.version)) {
    onAvailable(update);
  }
}
