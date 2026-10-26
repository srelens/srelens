import { describe, expect, it } from "vitest";
import type { ClusterContext } from "./clusters";
import {
  ASSISTANT_KEY,
  assistantAllowedFor,
  parseAssistantSetting,
  readAssistantSetting,
  writeAssistantSetting,
  type AssistantSetting,
} from "./assistantSetting";

const ON: AssistantSetting = { enabled: true, disabledClusters: [] };

const context = (name: string, sourceFile: string, original = name): ClusterContext => ({
  name,
  stableId: `${sourceFile}#${original}`,
  key: `${encodeURIComponent(sourceFile)}#${original}`,
  pinnedId: `srelens-context:${encodeURIComponent(sourceFile)}#${original}`,
  cluster: original,
  server: "https://example",
  isCurrent: false,
  sourceFile,
  authKind: "token",
});

const prod = context("prod", "C:/k/a");
const dev = context("dev", "C:/k/a");

describe("parseAssistantSetting", () => {
  it("reads nothing, garbage and the wrong shape as on with nothing disabled", () => {
    for (const raw of [null, "", "not json", "[]", "5", "null", '"on"']) {
      expect(parseAssistantSetting(raw)).toEqual(ON);
    }
  });

  it("reads an enabled flag that is not the boolean false as on, and drops a list that is not a list", () => {
    expect(parseAssistantSetting('{"enabled":"false","disabledClusters":"x"}')).toEqual(ON);
  });

  it("keeps a real off and only the string ids", () => {
    expect(parseAssistantSetting('{"enabled":false,"disabledClusters":["a#p",3,null]}')).toEqual({
      enabled: false,
      disabledClusters: ["a#p"],
    });
  });
});

describe("readAssistantSetting / writeAssistantSetting", () => {
  it("round-trips through a storage under srelens.assistant", () => {
    const map = new Map<string, string>();
    const storage = { getItem: (k: string) => map.get(k) ?? null, setItem: (k: string, v: string) => void map.set(k, v) };
    writeAssistantSetting({ enabled: false, disabledClusters: [prod.stableId] }, storage);
    expect(ASSISTANT_KEY).toBe("srelens.assistant");
    expect(JSON.parse(map.get(ASSISTANT_KEY)!)).toEqual({ enabled: false, disabledClusters: [prod.stableId] });
    expect(readAssistantSetting(storage)).toEqual({ enabled: false, disabledClusters: [prod.stableId] });
  });
});

describe("assistantAllowedFor", () => {
  const offForProd: AssistantSetting = { enabled: true, disabledClusters: [prod.stableId] };

  it("refuses a switched-off cluster by every name the app gives it", () => {
    for (const name of [prod.name, prod.stableId, prod.pinnedId, prod.key]) {
      expect(assistantAllowedFor(offForProd, [prod, dev], name)).toBe(false);
    }
  });

  it("allows another cluster, and no cluster at all, while global is on", () => {
    expect(assistantAllowedFor(offForProd, [prod, dev], "dev")).toBe(true);
    expect(assistantAllowedFor(offForProd, [prod, dev], "")).toBe(true);
    expect(assistantAllowedFor(offForProd, [prod, dev], undefined)).toBe(true);
  });

  it("refuses a cluster whose display name gained a file prefix", () => {
    const prefixed = context("a/prod", "C:/k/a", "prod");
    expect(assistantAllowedFor(offForProd, [prefixed], "a/prod")).toBe(false);
    expect(assistantAllowedFor(offForProd, [prefixed], "prod")).toBe(false);
  });

  it("refuses a switched-off stable id even when no listed context carries it", () => {
    expect(assistantAllowedFor(offForProd, [], prod.stableId)).toBe(false);
  });

  it("refuses everything while global is off", () => {
    const off: AssistantSetting = { enabled: false, disabledClusters: [] };
    for (const name of ["dev", "", undefined, null]) {
      expect(assistantAllowedFor(off, [prod, dev], name)).toBe(false);
    }
  });
});
