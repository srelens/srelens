import { Fragment } from "react";
import type { NoteBlock, NoteSpan } from "@srelens/core";

/**
 * The inline runs of one line. Plain text is a bare string rather than a
 * wrapper element: a `<span>` per run would say nothing to a reader or to
 * assistive technology, and it would put an element between `<code>` and the
 * words around it for no reason.
 */
function Spans({ spans }: { spans: NoteSpan[] }) {
  return (
    <>
      {spans.map((span, i) => {
        if (span.kind === "code") return <code key={i}>{span.text}</code>;
        if (span.kind === "strong") return <strong key={i}>{span.text}</strong>;
        return <Fragment key={i}>{span.text}</Fragment>;
      })}
    </>
  );
}

/**
 * Release-note blocks as prose.
 *
 * Content markup, not a component of the design system: headings, lists and
 * paragraphs are what the notes *are*, and the kit has no component that means
 * "a paragraph of someone else's text". The elements are the semantic ones so
 * the notes keep their outline — a reader navigating by heading finds the
 * sections — and `h3` because the screen's title is the `h1` and these sit
 * under it.
 *
 * Every span goes through React elements, never `dangerouslySetInnerHTML`, so
 * angle brackets in a hand-edited GitHub release body render as the characters
 * they are and cannot inject markup. The keys are indices because the blocks
 * are a pure function of the note text: nothing reorders them, and there is no
 * identity in them to key on.
 */
export function Notes({ blocks }: { blocks: NoteBlock[] }) {
  if (blocks.length === 0) return null;
  return (
    <div className="flex flex-col gap-2 text-[0.8125rem] leading-relaxed">
      {blocks.map((block, i) => {
        if (block.kind === "heading") {
          return (
            <h3 key={i} className="mt-1 text-[0.875rem] font-medium">
              <Spans spans={block.spans} />
            </h3>
          );
        }
        if (block.kind === "list") {
          return (
            <ul key={i} className="flex list-disc flex-col gap-1 pl-5">
              {block.items.map((item, j) => (
                <li key={j}>
                  <Spans spans={item} />
                </li>
              ))}
            </ul>
          );
        }
        return (
          <p key={i} className="text-muted">
            <Spans spans={block.spans} />
          </p>
        );
      })}
    </div>
  );
}
