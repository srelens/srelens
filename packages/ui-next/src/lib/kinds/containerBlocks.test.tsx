import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { PodContainer } from "@srelens/core";
import { ContainerBlocks, MAX_CONTAINER_BLOCKS } from "./containerBlocks";

const c = (over: Partial<PodContainer> = {}): PodContainer => ({
  name: "api", kind: "app", state: "running", reason: "", exitCode: null, ready: true, restarts: 0, image: "acme/api:1",
  ...over,
});
const blocks = () => screen.getAllByRole("img");
const state = (el: Element) => el.getAttribute("data-state");

afterEach(cleanup);

describe("ContainerBlocks", () => {
  it("draws one square per container, in the order it was given them", () => {
    render(<ContainerBlocks containers={[c({ name: "api" }), c({ name: "worker" }), c({ name: "proxy" })]} />);
    expect(blocks().map((b) => b.getAttribute("aria-label")?.split(":")[0])).toEqual(["api", "worker", "proxy"]);
  });

  it("gives each square the state core reads for its container", () => {
    render(
      <ContainerBlocks
        containers={[
          c({ name: "a" }),
          c({ name: "b", ready: false }),
          c({ name: "c", state: "waiting", reason: "ContainerCreating", ready: false }),
          c({ name: "d", state: "waiting", reason: "CrashLoopBackOff", ready: false }),
          c({ name: "e", state: "terminated", exitCode: 0, ready: false }),
          c({ name: "f", state: "terminated", exitCode: 137, reason: "OOMKilled", ready: false }),
          c({ name: "g", state: "unknown", ready: false }),
        ]}
      />,
    );
    expect(blocks().map(state)).toEqual(["ready", "unready", "starting", "stuck", "completed", "failed", "unknown"]);
  });

  it("names each square for its container, state, restarts and image, and says the same on hover", () => {
    render(<ContainerBlocks containers={[c({ name: "worker", state: "waiting", reason: "CrashLoopBackOff", ready: false, restarts: 14, image: "acme/worker:1" })]} />);
    const says = "worker: Waiting (CrashLoopBackOff), 14 restarts, acme/worker:1";
    expect(blocks()[0].getAttribute("aria-label")).toBe(says);
    expect(blocks()[0].getAttribute("title")).toBe(says);
  });

  /**
   * A restart is something that happened to a container, not what it is doing
   * now. It marks the square; it does not replace the square's state.
   */
  it("marks a container that has restarted, on top of what it is doing now", () => {
    render(
      <ContainerBlocks
        containers={[
          c({ name: "steady" }),
          c({ name: "bounced", restarts: 3 }),
          c({ name: "looping", state: "waiting", reason: "CrashLoopBackOff", ready: false, restarts: 14 }),
        ]}
      />,
    );
    const [steady, bounced, looping] = blocks();
    expect(steady.hasAttribute("data-restarted")).toBe(false);
    expect(bounced.hasAttribute("data-restarted")).toBe(true);
    expect(state(bounced)).toBe("ready");
    expect(looping.hasAttribute("data-restarted")).toBe(true);
    expect(state(looping)).toBe("stuck");
  });

  it("sets init containers apart, after the others, and draws a sidecar with the others", () => {
    const { container } = render(
      <ContainerBlocks
        containers={[
          c({ name: "api" }),
          c({ name: "migrate", kind: "init", state: "terminated", exitCode: 0, ready: false }),
          c({ name: "proxy", kind: "sidecar" }),
        ]}
      />,
    );
    const apart = container.querySelector(".ctr-blocks-init")!;
    expect([...apart.querySelectorAll("[role='img']")].map((b) => b.getAttribute("data-kind"))).toEqual(["init"]);
    // In the document: api, proxy, then the init container.
    expect(blocks().map((b) => b.getAttribute("aria-label")?.split(/[ :]/)[0])).toEqual(["api", "proxy", "migrate"]);
  });

  it("draws no separate init group for a pod with no init containers", () => {
    const { container } = render(<ContainerBlocks containers={[c()]} />);
    expect(container.querySelector(".ctr-blocks-init")).toBeNull();
  });

  it("keeps the ready count the old column printed, as the group's name and tooltip", () => {
    render(<ContainerBlocks containers={[c(), c({ name: "b", ready: false })]} />);
    const group = screen.getByRole("group", { name: "1 of 2 containers ready" });
    expect(group.getAttribute("title")).toBe("1 of 2 containers ready");
  });

  it("draws a fixed number of squares and counts the rest, so the column keeps its width", () => {
    const many = Array.from({ length: MAX_CONTAINER_BLOCKS + 3 }, (_, i) => c({ name: `c${i}` }));
    const { container } = render(<ContainerBlocks containers={many} />);
    expect(blocks()).toHaveLength(MAX_CONTAINER_BLOCKS);
    const more = container.querySelector(".ctr-more")!;
    expect(more.textContent).toBe("+3");
    // The ones not drawn are still named, on the count.
    expect(more.getAttribute("title")).toContain(`c${MAX_CONTAINER_BLOCKS}: Running`);
    // And the summary counts all of them, not only the ones drawn.
    expect(screen.getByRole("group").getAttribute("aria-label")).toBe(
      `${MAX_CONTAINER_BLOCKS + 3} of ${MAX_CONTAINER_BLOCKS + 3} containers ready`,
    );
  });

  /**
   * "Colour is never the only signal." A filled green square and a filled
   * orange one differ by colour, so whatever is not ordinary is said in a
   * word beside the squares as well.
   */
  it("says the worst container's reason in a word beside the squares", () => {
    const { container } = render(
      <ContainerBlocks containers={[c(), c({ name: "worker", state: "waiting", reason: "CrashLoopBackOff", ready: false })]} />,
    );
    expect(container.querySelector(".ctr-word")?.textContent).toBe("CrashLoopBackOff");
    cleanup();
    expect(render(<ContainerBlocks containers={[c({ ready: false })]} />).container.querySelector(".ctr-word")?.textContent).toBe(
      "Not ready",
    );
  });

  it("prints no word for a pod with nothing to report", () => {
    const done = c({ name: "migrate", kind: "init", state: "terminated", exitCode: 0, ready: false });
    const { container } = render(<ContainerBlocks containers={[c(), done]} />);
    expect(container.querySelector(".ctr-word")).toBeNull();
    expect(container.textContent).toBe("");
  });

  it("lets a keyboard reach the containers it did not draw", () => {
    const many = Array.from({ length: MAX_CONTAINER_BLOCKS + 2 }, (_, i) => c({ name: `c${i}` }));
    const { container } = render(<ContainerBlocks containers={many} />);
    const more = container.querySelector(".ctr-more") as HTMLElement;
    expect(more.tabIndex).toBe(0);
    more.focus();
    expect(document.activeElement).toBe(more);
    expect(more.getAttribute("aria-label")).toBe(
      `2 more: c${MAX_CONTAINER_BLOCKS}: Running, acme/api:1; c${MAX_CONTAINER_BLOCKS + 1}: Running, acme/api:1`,
    );
  });

  it("prints the fallback, or a dash, for a row that carries no containers", () => {
    expect(render(<ContainerBlocks containers={undefined} fallback="1/2" />).container.textContent).toBe("1/2");
    cleanup();
    expect(render(<ContainerBlocks containers={[]} fallback="0/0" />).container.textContent).toBe("0/0");
    cleanup();
    expect(render(<ContainerBlocks containers={undefined} />).container.textContent).toBe("—");
  });
});

/**
 * The look is the stylesheet's, and jsdom draws none of it. What can be held
 * here is that every state has a rule, that no two states are drawn the same,
 * and that the colours are tokens.
 */
describe("the container block styles", () => {
  const css = readFileSync(join(__dirname, "../../styles/next.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  const rule = (state: string) => css.match(new RegExp(`\\.ctr-block\\[data-state="${state}"\\]\\s*\\{([^}]*)\\}`))?.[1].trim();
  const states = ["ready", "unready", "starting", "stuck", "completed", "failed", "unknown"];

  it("has a rule for every state core can give", () => {
    for (const s of states) expect(rule(s), s).toBeTruthy();
  });

  it("draws no two states the same", () => {
    expect(new Set(states.map(rule)).size).toBe(states.length);
  });

  it("tells states apart by shape as well as by colour", () => {
    // The two greens differ by fill; the muted ones by fill, dash or motion.
    expect(rule("ready")).toContain("background");
    expect(rule("unready")).not.toContain("background");
    expect(rule("completed")).not.toContain("background");
    expect(rule("unknown")).toContain("dashed");
    expect(rule("starting")).toContain("animation");
  });

  it("uses the status tokens: green for running, orange for a back-off, red for a failure", () => {
    expect(rule("ready")).toContain("var(--ok)");
    expect(rule("stuck")).toContain("var(--warn)");
    expect(rule("failed")).toContain("var(--sev)");
    const all = css.slice(css.indexOf(".ctr-blocks"));
    expect(all).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });

  it("marks the two filled states that mean trouble, so they differ from ready by more than colour", () => {
    expect(css).toMatch(/\.ctr-block\[data-state="stuck"\]::before\s*\{\s*content: "!"/);
    expect(css).toMatch(/\.ctr-block\[data-state="failed"\]::before\s*\{\s*content: "×"/);
    // And the ordinary one carries none.
    expect(css).not.toMatch(/\.ctr-block\[data-state="ready"\]::before/);
  });

  it("shows where the keyboard is on the count of containers not drawn", () => {
    expect(css).toMatch(/\.ctr-more:focus-visible\s*\{[^}]*outline/);
  });

  it("stills the one animation for a reader who asked for less motion", () => {
    expect(css).toMatch(/prefers-reduced-motion: reduce\)\s*\{\s*\.ctr-block\[data-state="starting"\]\s*\{\s*animation: none/);
  });

  it("marks a restart with a dot, for every state alike", () => {
    expect(css).toMatch(/\.ctr-block\[data-restarted\]::after\s*\{/);
  });
});
