import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { JSDOM } from "jsdom";
const source = execFileSync(
  "tar",
  [
    "-xOzf",
    "../../crates/plugin-host/tests/fixtures/freelens-flux-5.3.1.tgz",
    "package/out/renderer/index.js",
  ],
  { maxBuffer: 2e6 },
).toString();
const settle = () => new Promise((r) => setTimeout(r, 100));
test("real upstream package mounts its list and registered GitRepository details", async () => {
  const dom = new JSDOM('<div id="root"></div>', {
    runScripts: "dangerously",
    url: "https://fixture.invalid",
  });
  const w = dom.window;
  w.TextEncoder = TextEncoder;
  w.eval(readFileSync("dist/runtime.js", "utf8"));
  const calls = [];
  const crds = [
    {
      group: "source.toolkit.fluxcd.io",
      plural: "gitrepositories",
      kind: "GitRepository",
      namespaced: true,
      version: "v1",
      versions: ["v1"],
    },
  ];
  const object = {
    apiVersion: "source.toolkit.fluxcd.io/v1",
    kind: "GitRepository",
    metadata: { name: "sample-repo", namespace: "flux-system", uid: "one" },
    spec: {
      url: "https://example.test/git",
      interval: "1m",
      ref: { branch: "main" },
    },
    status: {
      conditions: [
        { type: "Ready", status: "True", message: "stored artifact" },
      ],
      artifact: {
        revision: "main@sha1:0123456789abcdef",
        path: "gitrepository/flux-system/sample",
        url: "http://source/artifact",
      },
    },
  };
  w.FreelensRuntime.mount({
    source,
    crds,
    namespaces: ["flux-system"],
    page: "gitrepository",
    request: async (request) => {
      calls.push(request);
      return request.operation === "events"
        ? { events: [] }
        : { objects: [object] };
    },
  });
  await settle();
  assert.match(w.document.body.textContent, /sample-repo/);
  w.document.querySelector('[data-resource-name="sample-repo"]').click();
  await settle();
  assert.match(
    w.document.querySelector('[role="dialog"]').textContent,
    /https:\/\/example.test\/git/,
  );
  assert.match(
    w.document.querySelector('[role="dialog"]').textContent,
    /Interval/,
  );
  assert(calls.every((c) => !("context" in c)));
  w.close();
});
test("overview uses actual upstream dashboard, and failed reads remain visible", async () => {
  const dom = new JSDOM('<div id="root"></div>', {
    runScripts: "dangerously",
    url: "https://fixture.invalid",
  });
  try {
    const w = dom.window;
    w.TextEncoder = TextEncoder;
    w.eval(readFileSync("dist/runtime.js", "utf8"));
    w.FreelensRuntime.mount({
      source,
      crds: [
        {
          group: "source.toolkit.fluxcd.io",
          plural: "gitrepositories",
          kind: "GitRepository",
          versions: ["v1"],
        },
      ],
      namespaces: ["flux-system"],
      request: async () => {
        throw new Error("Forbidden: test RBAC");
      },
    });
    await settle();
    assert.match(w.document.body.textContent, /FluxCD Overview/);
    assert.match(
      w.document.querySelector('[role="alert"]').textContent,
      /Forbidden: test RBAC/,
    );
  } finally {
    dom.window.close();
  }
});
