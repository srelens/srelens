import { createElement, useMemo, useState, type ReactNode } from "react";
import { Button, CodeEditor, ErrorState, Table } from "@srelens/ui-kit";
import { parseAssistantMarkdown, type MdBlock } from "@srelens/core/lib/assistantMarkdown";
import type { NoteSpan } from "@srelens/core/lib/releaseNotes";
import { describeError } from "@srelens/core/lib/errors";
import { normalizeNativeComponentLink, openNativeComponentLink } from "@srelens/core/lib/nativeComponentLink";
import { NativeRows } from "./NativeRows";

function ExternalLink({ url, children }: { url: string; children: ReactNode }) {
  const [error, setError] = useState<unknown>();
  const open = async () => {
    setError(undefined);
    try { await openNativeComponentLink(url); } catch (cause) { setError(cause); }
  };
  return <>
    <a href={url} target="_blank" rel="noopener noreferrer" title={`Open external link: ${url}`} onClick={event => { event.preventDefault(); void open(); }}>{children}<span aria-hidden="true"> ↗</span><span className="sr-only"> (external link)</span></a>
    {error !== undefined && <span role="alert">Could not open external link: {describeError(error).detail} <Button type="button" variant="secondary" size="sm" onClick={() => void open()}>Retry link</Button></span>}
  </>;
}

/** A flat subset: links only in plain text, never code, images, HTML or nested markup. */
function links(text: string): ReactNode[] {
  const result: ReactNode[] = [];
  let from = 0;
  let search = 0;
  while (search < text.length) {
    const start = text.indexOf("[", search);
    if (start < 0) break;
    const middle = text.indexOf("](", start + 1);
    if (middle < 0) break;
    const end = text.indexOf(")", middle + 2);
    if (end < 0) break;
    // Nested brackets and images remain literal; skip this whole candidate.
    const title = text.slice(start + 1, middle);
    const url = normalizeNativeComponentLink(text.slice(middle + 2, end));
    if (url && title && !title.includes("[") && text[start - 1] !== "!") {
      result.push(text.slice(from, start));
      result.push(<ExternalLink key={`${start}:${url}`} url={url}>{title}</ExternalLink>);
      from = end + 1;
    }
    search = end + 1;
  }
  result.push(text.slice(from));
  return result;
}
function spans(values: NoteSpan[]) {
  return values.map((span, index) => span.kind === "strong" ? <strong key={index}>{span.text}</strong> : span.kind === "code" ? <code className="native-machine" key={index}>{span.text}</code> : <span key={index}>{links(span.text)}</span>);
}
function Block({ block }: { block: MdBlock }) {
  switch (block.kind) {
    case "heading": return createElement(`h${block.level}`, {}, spans(block.spans));
    case "paragraph": return <p>{spans(block.spans)}</p>;
    case "code": return <CodeEditor value={block.text} language="none" readOnly copy flush ariaLabel="Code block" minHeight={80} maxHeight={360}/>;
    case "bullet": return <NativeRows items={block.items}>{items => <ul>{items.map((item, index) => <li key={index}>{spans(item)}</li>)}</ul>}</NativeRows>;
    case "ordered": return <NativeRows items={block.items}>{items => <ol>{items.map((item, index) => <li key={index}>{spans(item)}</li>)}</ol>}</NativeRows>;
    case "table": return <NativeRows items={block.rows}>{rows => <Table columns={block.headers.map((header, index) => ({ key: String(index), header: spans(header), sortable: false, render: (row: NoteSpan[][]) => spans(row[index] ?? []) }))} data={rows} getRowKey={row => String(block.rows.indexOf(row))}/>}</NativeRows>;
  }
}
export function NativeMarkdown({ text }: { text: string }) {
  const blocks = useMemo(() => parseAssistantMarkdown(text), [text]);
  // Keep pathological table width bounded too, rather than silently dropping columns.
  if (blocks.some(block => block.kind === "table" && (block.headers.length > 20 || block.rows.some(row => row.length > 20)))) {
    return <ErrorState title="Could not display Markdown" detail="Markdown tables support at most 20 columns."/>;
  }
  return <div className="native-component-prose"><NativeRows items={blocks}>{visible => visible.map((block, index) => <Block key={index} block={block}/>)}</NativeRows></div>;
}
