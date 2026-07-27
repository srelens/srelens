import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// Unmount React trees between tests so repeated render() calls don't stack.
afterEach(() => cleanup());

// Web mode persists UI state (open tabs, namespaces, theme) to localStorage, so
// one test's writes must not leak into the next test's fresh render — clear it
// between tests. (Desktop is unaffected; this only matters under jsdom.)
afterEach(() => localStorage.clear());

// jsdom lacks a few browser APIs that HeroUI / React-Aria components touch
// (media queries for theming, ResizeObserver for popovers/overlays). Provide
// inert stubs so those components mount in tests.
if (!window.matchMedia) {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as unknown as MediaQueryList;
}

if (!("ResizeObserver" in window)) {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  (window as unknown as { ResizeObserver: unknown }).ResizeObserver = ResizeObserverStub;
}

// Radix popovers (Select, etc.) call these pointer-capture / scroll APIs that
// jsdom does not implement.
const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
proto.hasPointerCapture ??= () => false;
proto.setPointerCapture ??= () => {};
proto.releasePointerCapture ??= () => {};
proto.scrollIntoView ??= () => {};
