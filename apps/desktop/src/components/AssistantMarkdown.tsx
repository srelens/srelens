import { parseAssistantMarkdown } from "@srelens/core";
import type { NoteSpan } from "@srelens/core";

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
        if (block.kind === "heading") {
          const cls =
            block.level <= 1
              ? "text-base font-semibold"
              : block.level === 2
                ? "text-[0.95rem] font-semibold"
                : "text-sm font-semibold text-muted-foreground";
          return (
            <p key={i} className={`${cls} mt-1`}>
              <Spans spans={block.spans} />
            </p>
          );
        }
        if (block.kind === "table") {
          return (
            <div key={i} className="overflow-x-auto">
              <table className="w-full border-collapse text-xs">
                <thead>
                  <tr>
                    {block.headers.map((h, j) => (
                      <th key={j} className="border border-border bg-muted/40 px-2 py-1 text-left font-semibold">
                        <Spans spans={h} />
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {block.rows.map((row, r) => (
                    <tr key={r}>
                      {row.map((cell, c) => (
                        <td key={c} className="border border-border px-2 py-1 align-top">
                          <Spans spans={cell} />
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
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
