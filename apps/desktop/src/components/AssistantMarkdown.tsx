import { parseAssistantMarkdown } from "../lib/assistantMarkdown";
import type { NoteSpan } from "../lib/releaseNotes";

// Renders an assistant chat reply as real markdown (bold, inline code,
// bullets, numbered lists, fenced code) instead of dumping raw `**`/`` ` ``/`-`
// syntax into a `whitespace-pre-wrap` block. Everything goes through React
// elements — no dangerouslySetInnerHTML — so reply text can never inject
// markup, matching the pattern `ReleaseNotes.tsx` uses for update notes.

function Spans({ spans }: { spans: NoteSpan[] }) {
  return (
    <>
      {spans.map((span, i) => {
        if (span.kind === "code") {
          return (
            <code key={i} className="rounded bg-foreground/10 px-1 py-0.5 font-mono text-[0.85em]">
              {span.text}
            </code>
          );
        }
        if (span.kind === "strong") return <strong key={i}>{span.text}</strong>;
        return <span key={i}>{span.text}</span>;
      })}
    </>
  );
}

export function AssistantMarkdown({ text }: { text: string }) {
  const blocks = parseAssistantMarkdown(text);
  return (
    <div className="space-y-2">
      {blocks.map((block, i) => {
        if (block.kind === "bullet") {
          return (
            <ul key={i} className="list-disc space-y-1 pl-4">
              {block.items.map((item, j) => (
                <li key={j}>
                  <Spans spans={item} />
                </li>
              ))}
            </ul>
          );
        }
        if (block.kind === "ordered") {
          return (
            <ol key={i} className="list-decimal space-y-1 pl-4">
              {block.items.map((item, j) => (
                <li key={j}>
                  <Spans spans={item} />
                </li>
              ))}
            </ol>
          );
        }
        if (block.kind === "code") {
          return (
            <pre key={i} className="overflow-x-auto rounded bg-foreground/10 p-2 font-mono text-xs">
              <code>{block.text}</code>
            </pre>
          );
        }
        return (
          <p key={i}>
            <Spans spans={block.spans} />
          </p>
        );
      })}
    </div>
  );
}
