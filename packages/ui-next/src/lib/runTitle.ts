/** Openers that say nothing about the question — dropped so the title starts
 *  at the subject. Ordered longest-first so "can you please" goes before
 *  "can you". */
const FILLER = [
  "can you please",
  "could you please",
  "can you",
  "could you",
  "please",
  "hey",
  "hi",
  "i want to",
  "i need to",
  "i would like to",
  "tell me",
  "show me",
  "help me",
];

const MAX = 56;

/**
 * A readable title for a conversation, from the question that opened it.
 *
 * The rail listed the raw question at 120 characters, which truncated
 * mid-word — `What is m01-prod-04-mongodb-0 usin…` — and repeated whatever
 * filler the reader had typed.
 *
 * **Derived, not generated.** Asking the agent to write a title would cost a
 * model call per conversation, could not be done for the sessions already on
 * disk, and would put a sentence on screen that srelens cannot attribute to
 * anything the reader said. This is the reader's own words, tidied: filler
 * dropped, whitespace collapsed, cut at a WORD boundary, first letter raised,
 * trailing punctuation removed.
 */
export function titleFromQuestion(question: string): string {
  let text = question.replace(/\s+/g, " ").trim();
  if (text === "") return "";

  // Filler only at the START, and only once: "show me the pods that show me"
  // is a real sentence and the second half is not an opener.
  const lower = text.toLowerCase();
  for (const opener of FILLER) {
    // The trailing space matters: "hi" must not eat the front of "history".
    if (lower.startsWith(`${opener} `)) {
      text = text.slice(opener.length + 1).trim();
      break;
    }
    // And a question that is ONLY an opener has no subject at all.
    if (lower === opener) {
      text = "";
      break;
    }
  }
  if (text === "") return "";

  // Trailing punctuation goes, but a question mark stays: it is part of how the
  // question reads, and a title ending in one still reads as a question.
  text = text.replace(/[.,;:\s]+$/, "");

  if (text.length > MAX) {
    const cut = text.slice(0, MAX);
    const lastSpace = cut.lastIndexOf(" ");
    // Only break at a space if there IS one late enough to leave a title —
    // otherwise one very long token would collapse to almost nothing.
    text = `${(lastSpace > MAX * 0.6 ? cut.slice(0, lastSpace) : cut).replace(/[.,;:]+$/, "")}…`;
  }

  return text.charAt(0).toUpperCase() + text.slice(1);
}
