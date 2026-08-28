import { parseReleaseNotes, type NoteSpan } from "@srelens/core";

// Renders the update dialog's release notes as real headings and lists instead
// of dumping the raw markdown into a <pre>. Everything goes through React
// elements — no dangerouslySetInnerHTML — so note text can never inject markup.

function Spans({ spans }: { spans: NoteSpan[] }) {
  return (
    <>
      {spans.map((span, i) => {
        if (span.kind === "code") return <code key={i}>{span.text}</code>;
        if (span.kind === "strong") return <strong key={i}>{span.text}</strong>;
        return <span key={i}>{span.text}</span>;
      })}
    </>
  );
}

export function ReleaseNotes({ notes }: { notes: string }) {
  const blocks = parseReleaseNotes(notes);
  if (!blocks.length) return null;
  return (
    <div className="fl-settings-update__notes">
      {blocks.map((block, i) => {
        if (block.kind === "heading") {
          return (
            <h4 key={i} className="fl-release-notes__heading">
              <Spans spans={block.spans} />
            </h4>
          );
        }
        if (block.kind === "list") {
          return (
            <ul key={i} className="fl-release-notes__list">
              {block.items.map((item, j) => (
                <li key={j}>
                  <Spans spans={item} />
                </li>
              ))}
            </ul>
          );
        }
        return (
          <p key={i} className="fl-release-notes__para">
            <Spans spans={block.spans} />
          </p>
        );
      })}
    </div>
  );
}
