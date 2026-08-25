import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock } = vi.hoisted(() => ({ invokeCommandMock: vi.fn() }));
vi.mock("../transport/transport", () => ({ invokeCommand: invokeCommandMock }));

const platform = vi.hoisted(() => ({ isTauri: vi.fn(() => true) }));
vi.mock("../transport/platform", () => ({
  isTauri: platform.isTauri,
  get isWeb() {
    return !platform.isTauri();
  },
}));

import { browsable, openExternal } from "./openExternal";

let opened: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  platform.isTauri.mockReturnValue(true);
  invokeCommandMock.mockResolvedValue(null);
  opened = vi.fn();
  Object.defineProperty(window, "open", { value: opened, configurable: true, writable: true });
});

describe("browsable", () => {
  it("gives a bare authority the scheme a browser needs", () => {
    // `forwardAddress` answers `localhost:12492` on the desktop, which is not
    // a URL: opened verbatim it is a relative path or a search term.
    expect(browsable("localhost:12492")).toBe("http://localhost:12492");
    expect(browsable("127.0.0.1:8080")).toBe("http://127.0.0.1:8080");
  });

  it("leaves an absolute URL exactly as it found it", () => {
    expect(browsable("http://localhost:9090/metrics")).toBe("http://localhost:9090/metrics");
    // And never downgrades one: a proxy served over TLS stays over TLS.
    expect(browsable("https://srelens.example/pf/7/")).toBe("https://srelens.example/pf/7/");
    expect(browsable("HTTPS://srelens.example/pf/7/")).toBe("HTTPS://srelens.example/pf/7/");
  });
});

describe("openExternal — the desktop", () => {
  it("hands the URL to the Rust command, because the WebView opens nothing", async () => {
    // `window.open` and `<a target="_blank">` are silent no-ops in a Tauri
    // WebView (#348): wry's new-window delegate returns nil unless a handler
    // is installed, and srelens installs none.
    await openExternal("http://localhost:12492");
    expect(invokeCommandMock).toHaveBeenCalledWith("open_external", {
      url: "http://localhost:12492",
    });
    expect(opened).not.toHaveBeenCalled();
  });

  it("reports what the backend refused, rather than swallowing it", async () => {
    invokeCommandMock.mockRejectedValue(new Error("no default browser"));
    await expect(openExternal("http://localhost:12492")).rejects.toThrow(/no default browser/);
  });
});

describe("openExternal — web mode", () => {
  it("opens a tab, where the page really is in a browser", async () => {
    platform.isTauri.mockReturnValue(false);
    await openExternal(`${window.location.origin}/pf/7/`);
    // The mechanism, not the string: jsdom's own origin contains "localhost",
    // so a web assertion written against the address alone would still hold
    // for a desktop-shaped answer.
    expect(opened).toHaveBeenCalledWith(
      `${window.location.origin}/pf/7/`,
      "_blank",
      "noopener,noreferrer",
    );
    expect(invokeCommandMock).not.toHaveBeenCalled();
  });

  it("does not read `window.open`'s return value", async () => {
    // With `noopener` the spec says the call returns null EVEN WHEN THE TAB
    // OPENED. A null check here would reject every successful web open.
    platform.isTauri.mockReturnValue(false);
    opened.mockReturnValue(null);
    await expect(openExternal("https://srelens.example/pf/7/")).resolves.toBeUndefined();
  });
});

describe("openExternal — what it refuses", () => {
  const refused = [
    "javascript:alert(1)",
    "file:///etc/passwd",
    "data:text/html,<script>alert(1)</script>",
    // The bare authority itself: `browsable` is the caller's job, and a
    // command that quietly fixed it up would be a general "open any string".
    "localhost:12492",
    "/pf/7/",
    "",
  ];

  for (const url of refused) {
    it(`refuses ${JSON.stringify(url)} on both platforms`, async () => {
      for (const desktop of [true, false]) {
        vi.clearAllMocks();
        platform.isTauri.mockReturnValue(desktop);
        await expect(openExternal(url)).rejects.toThrow(/http/i);
        expect(invokeCommandMock).not.toHaveBeenCalled();
        expect(opened).not.toHaveBeenCalled();
      }
    });
  }
});
