/**
 * Pasting a screenshot into the prompt.
 *
 * The shape is classic's (`AssistantConversation.tsx`), reimplemented here
 * rather than imported: `apps/desktop` depends on this package, so importing
 * the other way would invert that. The three functions are small and the
 * behaviour has to match, so the comments say what each half is for.
 *
 * The new design never had this at all — a claim I made twice while deleting
 * `Composer` was that it "dropped image attachments", and that was wrong:
 * `Composer` had no image handling either. It was simply never built.
 */

/** The image files out of a paste, if it carries any. Everything else — plain
 *  text, HTML — is left for the input's ordinary paste handling, so a
 *  text paste is untouched. */
export function extractImageFiles(clipboardData: DataTransfer | null | undefined): File[] {
  if (!clipboardData) return [];
  return Array.from(clipboardData.files ?? []).filter((f) => f.type.startsWith("image/"));
}

/**
 * A `File`'s bytes as a base64 data URI.
 *
 * A data URI, not raw base64, because this is what gets DISPLAYED — a
 * thumbnail beside the prompt and, once sent, on the turn itself. The raw
 * payload the backend wants is taken from it at the last moment; see
 * {@link stripDataUri}.
 */
export function readImageFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("failed to read the pasted image"));
    reader.readAsDataURL(file);
  });
}

/**
 * The raw base64 out of a data URI.
 *
 * `chat_send` hands its `images` straight to `decode_base64_image`, which is
 * `STANDARD.decode` (`assistant.rs:245-249`) — it does not strip a
 * `data:image/png;base64,` prefix, it fails on it. So the URI is what srelens
 * shows and this is what srelens sends.
 *
 * A string with no comma is returned unchanged: already-raw base64 is a legal
 * input, and guessing otherwise would corrupt it.
 */
export function stripDataUri(uri: string): string {
  const i = uri.indexOf(",");
  return i === -1 ? uri : uri.slice(i + 1);
}
