import { describe, it, expect, vi } from "vitest";
import type { ReactElement } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { eventVerdict, type EventSummary } from "@srelens/core";
import type { Column } from "@srelens/ui-kit";
import {
  EVENT_DESCRIPTOR,
  eventAskQuestion,
  eventColumns,
  eventNamespace,
  withEventAsk,
  type EventRow,
} from "./events";

const event = (over: Partial<EventRow> = {}): EventRow => ({
  name: "shop/web-0.17a",
  namespace: "shop",
  type: "Warning",
  reason: "BackOff",
  object: "Pod/web-0",
  message: "Back-off restarting failed container checkout in pod web-0",
  count: 37,
  age: "12s",
  ...over,
});

const column = (key: string): Column<EventRow> =>
  eventColumns.find((c) => c.key === key) ?? expect.fail(`no ${key} column`);

/** The rendered cell as a DOM element, the way `columns.test.ts` reads one. */
const cell = (key: string, row: EventRow) => render(column(key).render!(row) as ReactElement).container;

describe("the events descriptor", () => {
  it("draws the design's eight columns, in order, with the design's headers", () => {
    const decorated = withEventAsk(EVENT_DESCRIPTOR.columns, () => {});
    expect(decorated.map((c) => c.key)).toEqual([
      "type", "reason", "object", "namespace", "message", "count", "age", "ask",
    ]);
    expect(decorated.map((c) => c.header)).toEqual([
      "Type", "Reason", "Object", "Namespace", "Message", "Count", "Age", "",
    ]);
  });

  it("streams from the watch, is namespaced, and offers no action on an event", () => {
    expect(EVENT_DESCRIPTOR.source).toBe("watch");
    expect(EVENT_DESCRIPTOR.scope).toBe("namespaced");
    expect(EVENT_DESCRIPTOR.k8sKind).toBe("Event");
    expect(EVENT_DESCRIPTOR.actions).toEqual({});
    expect(EVENT_DESCRIPTOR.load).toBeUndefined();
  });

  it("gives an event no health of its own, so no row shows a dot that is always off", () => {
    expect(EVENT_DESCRIPTOR.flagged).toBeUndefined();
  });

  it("takes the watch's own payload as a row, field for field", () => {
    const summary: EventSummary = {
      name: "shop/web-0.17a", namespace: "shop", type: "Normal", reason: "Pulled",
      object: "Pod/web-0", message: "already present", age: "4m", count: 1,
    };
    const row: EventRow = summary;
    expect(row.count).toBe(1);
    expect(row.namespace).toBe("shop");
  });
});

describe("the Type cell", () => {
  // Driven by `eventVerdict` rather than by literals written here: an
  // implementation that branched on "Warning" itself would pass a pair of
  // hand-written expectations and drift the day core's rule changes.
  it.each(["Warning", "Normal", "Something"])("tones %s from eventVerdict, not from a literal", (type) => {
    const { health, bad } = eventVerdict(type);
    const pill = cell("type", event({ type })).querySelector(".status");
    expect(pill).not.toBeNull();
    expect(pill?.getAttribute("data-kind")).toBe(health);
    expect(pill?.getAttribute("data-bad")).toBe(bad ? "true" : null);
    expect(pill?.textContent).toBe(type);
  });

  it("never draws a bare dot: an event with no type reads as an em dash", () => {
    expect(cell("type", event({ type: "" })).querySelector(".status")?.textContent).toBe("—");
  });
});

describe("the Object cell", () => {
  it("lowercases the kind and keeps the name, so Pod/web-1 reads pod/web-1", () => {
    expect(cell("object", event({ object: "Pod/web-1" })).textContent).toBe("pod/web-1");
    expect(cell("object", event({ object: "StatefulSet/Web-1" })).textContent).toBe("statefulset/Web-1");
  });

  it("leaves a value with no kind in front of it alone", () => {
    expect(cell("object", event({ object: "web-1" })).textContent).toBe("web-1");
  });

  it("truncates at the design's 220 px, and gives the full value back on hover", () => {
    const span = cell("object", event()).querySelector("span");
    expect(span?.className).toContain("truncate");
    expect(span?.style.maxWidth).toBe("220px");
    expect(span?.getAttribute("title")).toBe("pod/web-0");
  });
});

describe("the Namespace cell", () => {
  it("reads the namespace the row carries", () => {
    expect(cell("namespace", event({ namespace: "checkout" })).textContent).toBe("checkout");
  });

  it("reads the field, and never splits the key, however the key is spelled", () => {
    // The row key stays `<namespace>/<name>` — it has a job, and this no longer
    // reads anything out of it. A row whose key and field disagree is the only
    // way to tell the two apart, so it is the assertion worth making.
    expect(eventNamespace(event({ name: "shop/web-0.17a", namespace: "checkout" }))).toBe("checkout");
    expect(cell("namespace", event({ name: "shop/web-0.17a", namespace: "checkout" })).textContent).toBe("checkout");
    expect(eventNamespace(event({ name: "web-0.17a", namespace: "shop" }))).toBe("shop");
  });

  it("shows an em dash for a cluster-scoped event, never the word undefined", () => {
    expect(cell("namespace", event({ name: "node-a.17b", namespace: "" })).textContent).toBe("—");
    expect(eventNamespace(event({ name: "node-a.17b", namespace: "" }))).toBe("");
  });

  it("sorts and filters on the namespace it shows", () => {
    expect(column("namespace").getValue!(event({ namespace: "shop" }))).toBe("shop");
  });
});

describe("the Message cell", () => {
  it("shows the whole message on hover and truncates what it draws", () => {
    const row = event();
    const span = cell("message", row).querySelector("span");
    expect(span?.textContent).toBe(row.message);
    expect(span?.getAttribute("title")).toBe(row.message);
    expect(span?.className).toContain("truncate");
  });

  it("caps what it draws, instead of pushing Count and Age off the table", () => {
    // `Table` measures a column's natural width once and pins it, and `.tbl td`
    // is `white-space: nowrap` — so an UNCAPPED message cell measures as the
    // whole message (130 characters on the demo cluster) and the two columns
    // behind it are pushed out of the container: the repeat count a whole
    // backend task added was invisible on a 1600 px window. `truncate` alone
    // never fires, because nothing ever makes the box narrower than its text.
    const span = cell("message", event({ message: "the readiness probe failed ".repeat(5) })).querySelector("span");
    const message = Number.parseInt(span?.style.maxWidth ?? "", 10);
    expect(message).toBeGreaterThan(0);
    // And it is the widest cap in the table: Message is the column §8 gives the
    // slack to, and a Message narrower than the Object beside it would read as
    // the design's own order inverted.
    const object = Number.parseInt(
      cell("object", event()).querySelector("span")?.style.maxWidth ?? "",
      10,
    );
    expect(message).toBeGreaterThan(object);
  });

  it("does not offer to sort a paragraph of prose", () => {
    expect(column("message").sortable).toBe(false);
  });
});

describe("Count and Age", () => {
  it("shows how many times the event fired, right-aligned with the age", () => {
    expect(cell("count", event({ count: 37 })).textContent).toBe("37");
    expect(column("count").align).toBe("end");
    expect(column("age").align).toBe("end");
  });

  it("sorts the age by the row's own duration, so 1y outranks 300d", () => {
    const age = column("age");
    expect(Number(age.getSortValue!(event({ age: "1y" })))).toBeGreaterThan(
      Number(age.getSortValue!(event({ age: "300d" }))),
    );
  });

  it("renders the age the row carries", () => {
    expect(cell("age", event({ age: "12s" })).textContent).toBe("12s");
  });
});

describe("the hover ask", () => {
  it("asks the design's question, naming the reason and the message", () => {
    expect(eventAskQuestion(event())).toBe(
      "Explain this event: BackOff — Back-off restarting failed container checkout in pod web-0",
    );
  });

  it("hands that question to the console when the chip is used", async () => {
    const ask = vi.fn();
    const askColumn = withEventAsk(EVENT_DESCRIPTOR.columns, ask).at(-1)!;
    render(askColumn.render!(event()) as ReactElement);
    await userEvent.click(screen.getByRole("button"));
    expect(ask).toHaveBeenCalledWith(eventAskQuestion(event()));
  });

  it("stays out of the search and out of the sort, like every other affordance", () => {
    const askColumn = withEventAsk(EVENT_DESCRIPTOR.columns, () => {}).at(-1)!;
    expect(askColumn.sortable).toBe(false);
    expect(askColumn.filterable).toBe(false);
  });

  it("leaves the descriptor's own columns undecorated, so the picker never offers it", () => {
    expect(EVENT_DESCRIPTOR.columns.map((c) => c.key)).not.toContain("ask");
  });
});
