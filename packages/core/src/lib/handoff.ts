/** The newest turns a handoff keeps, and how much of each. Together they are
 *  the bound: at most ~12k characters, however long the conversation. */
const HANDOFF_TURNS = 10;
const HANDOFF_CHARS = 1200;

/**
 * The visible conversation, for an agent that has no record of it.
 *
 * Switching agent drops `resume` — one CLI's conversation id must never reach
 * another, and switching back must not resume a thread that skipped the other
 * agent's turns — so the agent switched to starts with nothing, and a
 * follow-up like "which index should I create?" got "which database do you
 * mean?". Prepended to the first question it is asked instead, until it has a
 * resume id of its own.
 *
 * The reader's questions and the final answers only: no tool calls, results
 * or thoughts. `role` is ui-next's `"agent"` or classic's `"assistant"`; error
 * turns are left out. Returns `""` when there is nothing to hand over.
 */
export function handoffPreface(turns: readonly { role: string; text: string }[]): string {
  const said = turns
    .filter((t) => (t.role === "user" || t.role === "agent" || t.role === "assistant") && t.text.trim() !== "")
    .slice(-HANDOFF_TURNS)
    .map((t) => {
      const text = t.text.trim();
      const clipped = text.length > HANDOFF_CHARS ? `${text.slice(0, HANDOFF_CHARS)} […]` : text;
      return `${t.role === "user" ? "Reader" : "Agent"}: ${clipped}`;
    });
  if (said.length === 0) return "";
  return (
    "The reader was talking to another agent before you, and you have no record of that conversation. " +
    "Here it is for context, oldest first; answer only their new question below it.\n\n" +
    `<earlier-conversation>\n${said.join("\n\n")}\n</earlier-conversation>\n\n`
  );
}
