import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { describeError } from "@srelens/core";
import { FailureAlert, FailureState, FailureWord, friendly, summarise } from "./errorCopy";

/** The 401 the overview's Fleet rail was printing at the reader, verbatim. */
const API_401 =
  'Error: handler error: ApiError: Unauthorized: Unauthorized (Status { status: Some("Failure"), ' +
  "metadata: Some(ListMeta { continue_: None, remaining_item_count: None, resource_version: None, " +
  'self_link: None }), reason: Some("Unauthorized"), code: Some(401), message: Some("Unauthorized") })';

/** `redactSecretManifest`'s fail-closed copy: this package's own words. */
const REDACTION_REFUSED =
  "This Secret's manifest is not shown, because it could not be redacted: it uses YAML aliases, " +
  "which could re-expose a redacted value.";

describe("friendly", () => {
  it("classifies rather than repeating what the cluster said", () => {
    const copy = friendly(API_401);
    expect(copy.title).toBe("Not authorized");
    expect(copy.detail).toMatch(/rejected your credentials/);
    expect(copy.detail).not.toMatch(/ListMeta/);
  });

  it("keeps the original when the copy replaced it", () => {
    expect(friendly(API_401).raw).toContain("ListMeta");
  });

  it("offers no original when the copy IS the original", () => {
    // The generic case returns the cleaned message as the detail. A
    // disclosure that opens onto the line above it costs a click and teaches
    // nothing.
    const copy = friendly("something nobody has classified");
    expect(copy.detail).toBe("something nobody has classified");
    expect(copy.raw).toBeUndefined();
  });

  it("passes this package's own refusals through as written", () => {
    // `redactSecretManifest` fails closed with sentences chosen so that no
    // error message quotes the Secret's source. They are not cluster errors
    // and match no branch, so they arrive whole — and, crucially, the SCREEN
    // keeps its own title, so the reader is never told "Something went wrong"
    // about the most careful message this codebase writes.
    const copy = friendly(REDACTION_REFUSED);
    expect(copy.detail).toBe(REDACTION_REFUSED);
    expect(copy.raw).toBeUndefined();
  });
});

describe("summarise", () => {
  it("says one outage once", () => {
    // Six kinds refused by one expired token is one problem, not six.
    const { detail } = summarise([API_401, API_401, API_401]);
    expect(detail).toBe(describeError(API_401).detail);
  });

  it("keeps genuinely different failures apart", () => {
    const { detail } = summarise([API_401, "dial tcp 10.1.2.3:6443: connect: connection refused"]);
    expect(detail).toMatch(/rejected your credentials/);
    expect(detail).toMatch(/connection to the API server could not be made/);
    expect(detail).toContain(" · ");
  });

  it("drops the empty reasons the callers filter on", () => {
    expect(summarise([]).detail).toBe("");
    expect(summarise(["", ""]).detail).toBe("");
  });

  it("carries every original, separated so two structs cannot read as one", () => {
    const raw = summarise([API_401, "x509: certificate signed by unknown authority"]).raw ?? "";
    expect(raw).toContain("ListMeta");
    expect(raw).toContain("x509");
    expect(raw.split("\n\n")).toHaveLength(2);
  });
});

describe("FailureState", () => {
  it("keeps the screen's own title and replaces only the detail", () => {
    const { container } = render(
      <FailureState title="Could not list pods on prod-eu" error={API_401} />,
    );
    expect(screen.getByText("Could not list pods on prod-eu")).toBeDefined();
    expect(screen.getByText(/rejected your credentials/)).toBeDefined();
    // The struct is reachable (see below) but it is not the copy: the two
    // slots the reader actually reads carry none of it.
    expect(container.querySelector('[data-slot="detail"]')?.textContent).not.toMatch(/ListMeta/);
    expect(screen.getByRole("alert").firstElementChild?.textContent).not.toMatch(/ListMeta/);
  });

  it("still has the original, folded away", () => {
    const { container } = render(<FailureState title="Could not list pods" error={API_401} />);
    const disclosure = container.querySelector('[data-slot="raw"]') as HTMLDetailsElement;
    expect(disclosure.open).toBe(false);
    expect(disclosure.textContent).toContain("ListMeta");
  });

  it("falls back to the classification's headline when the screen has no title of its own", () => {
    render(<FailureState error={API_401} />);
    expect(screen.getByText("Not authorized")).toBeDefined();
  });

  it("prints a Secret redaction refusal exactly as it was written", () => {
    render(
      <FailureState title="Could not load secret/db-creds's manifest" error={REDACTION_REFUSED} />,
    );
    expect(screen.getByText("Could not load secret/db-creds's manifest")).toBeDefined();
    expect(screen.getByText(REDACTION_REFUSED)).toBeDefined();
    expect(screen.queryByText("Something went wrong")).toBeNull();
  });
});

describe("FailureAlert", () => {
  it("keeps the caller's sentence about the rows and rewrites only the reason", () => {
    render(<FailureAlert title="These pods are stale" error={API_401} />);
    expect(screen.getByText("These pods are stale")).toBeDefined();
    expect(screen.getByText(/rejected your credentials/)).toBeDefined();
  });

  it("is a warning over surviving rows, not an alert that stops the screen", () => {
    const { container } = render(<FailureAlert title="These pods are stale" error={API_401} />);
    expect(container.querySelector('[role="status"]')).not.toBeNull();
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it("carries the original under it", () => {
    const { container } = render(<FailureAlert title="These pods are stale" error={API_401} />);
    expect(container.querySelector('[data-slot="raw"]')?.textContent).toContain("ListMeta");
  });
});

describe("FailureWord", () => {
  it("is the headline and nothing else, for a surface with no room for a paragraph", () => {
    render(<FailureWord error={API_401} />);
    expect(screen.getByText("Not authorized")).toBeDefined();
    expect(screen.queryByText(/rejected your credentials/)).toBeNull();
  });

  it("still reaches the original, which is the whole reason it is not a title attribute", () => {
    const { container } = render(<FailureWord error={API_401} />);
    const disclosure = container.querySelector('[data-slot="raw"]') as HTMLDetailsElement;
    expect(disclosure.open).toBe(false);
    expect(disclosure.textContent).toContain("ListMeta");
    for (const node of Array.from(container.querySelectorAll("*"))) {
      for (const attribute of Array.from(node.attributes)) {
        expect(attribute.value).not.toContain("ListMeta");
      }
    }
  });

  it("puts a disclosure in flow markup, not inside phrasing content", () => {
    // `details` is flow content. Nested in a `span` the parser rewrites the
    // tree underneath the component, which is how a row loses its layout on
    // one browser and keeps it on another.
    const { container } = render(<FailureWord error={API_401} />);
    expect((container.firstElementChild as HTMLElement).tagName).toBe("DIV");
  });

  it("still offers the original when nothing classified it", () => {
    // The narrow surfaces print the TITLE, and an unclassified failure's title
    // is "Something went wrong" — the message itself is only in `raw`. A row
    // that said "Something went wrong" and stopped there would be telling the
    // reader LESS than the raw string it replaced, which is the one thing this
    // change is not allowed to do.
    const { container } = render(<FailureWord error="etcdserver: leader changed" />);
    expect(screen.getByText(/Something went wrong/)).toBeDefined();
    const disclosure = container.querySelector('[data-slot="raw"]') as HTMLDetailsElement;
    expect(disclosure).not.toBeNull();
    expect(disclosure.textContent).toContain("etcdserver: leader changed");
  });

  it("takes a lead so a row can say what it is about", () => {
    render(<FailureWord error={API_401} lead="Could not count Pod: " />);
    expect(screen.getByText(/Could not count Pod: Not authorized/)).toBeDefined();
  });
});
