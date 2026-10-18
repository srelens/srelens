import { beforeEach, describe, expect, it } from "vitest";
import {
  __resetConsoleLayoutForTests,
  CONSOLE_HEIGHT_KEY,
  CONSOLE_PAGE_ROOM,
  consoleHeight,
  consoleHeightBounds,
  consoleMaximized,
  consoleSpareAbove,
  MIN_PAGE_ROOM,
  MIN_CONSOLE_HEIGHT,
  parseStoredConsoleHeight,
  saveConsoleHeight,
  setConsoleMaximized,
} from "./consoleLayout";

function memory() {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
    removeItem: (k: string) => void map.delete(k),
    map,
  };
}

beforeEach(() => __resetConsoleLayoutForTests());

describe("the console's height", () => {
  it("is the content's until the reader drags it", () => {
    expect(consoleHeight(memory())).toBeNull();
  });

  it("is kept between launches, never below the minimum", () => {
    const storage = memory();
    saveConsoleHeight(310.4, storage);
    expect(storage.map.get(CONSOLE_HEIGHT_KEY)).toBe("310");
    __resetConsoleLayoutForTests();
    expect(consoleHeight(storage)).toBe(310);
    saveConsoleHeight(10, storage);
    expect(consoleHeight(storage)).toBe(MIN_CONSOLE_HEIGHT);
  });

  it("reads anything but a height as nothing saved", () => {
    expect(parseStoredConsoleHeight(null)).toBeNull();
    expect(parseStoredConsoleHeight("tall")).toBeNull();
    expect(parseStoredConsoleHeight('"300"')).toBeNull();
    expect(parseStoredConsoleHeight("12")).toBeNull();
    expect(parseStoredConsoleHeight("300")).toBe(300);
  });

  it("always leaves the page its room, on any window", () => {
    expect(consoleHeightBounds(1000)).toEqual({ min: MIN_CONSOLE_HEIGHT, max: 1000 - CONSOLE_PAGE_ROOM });
    expect(consoleHeightBounds(200).max).toBe(MIN_CONSOLE_HEIGHT);
  });

  it("survives a storage that refuses", () => {
    const refusing = {
      getItem: () => {
        throw new Error("disabled");
      },
      setItem: () => {
        throw new Error("disabled");
      },
      removeItem: () => {},
    };
    const quiet = console.error;
    console.error = () => {};
    try {
      expect(consoleHeight(refusing)).toBeNull();
      saveConsoleHeight(300, refusing);
      expect(consoleHeight(refusing)).toBe(300);
    } finally {
      console.error = quiet;
    }
  });
});

describe("a maximized console", () => {
  it("is a state of this window only", () => {
    expect(consoleMaximized()).toBe(false);
    setConsoleMaximized(true);
    expect(consoleMaximized()).toBe(true);
    __resetConsoleLayoutForTests();
    expect(consoleMaximized()).toBe(false);
  });
});

describe("the room above the console (#948)", () => {
  it("is the content area less the floor a page needs", () => {
    expect(consoleSpareAbove(500)).toBe(500 - MIN_PAGE_ROOM);
    expect(consoleSpareAbove(100)).toBeLessThan(0);
  });
});
