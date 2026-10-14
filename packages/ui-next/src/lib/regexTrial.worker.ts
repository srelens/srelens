import { runTrial } from "./regexTrial";

// Typed by hand, not with the `webworker` lib: that lib redeclares globals the
// DOM lib already has, so it cannot share a tsconfig with the rest of this package.
const scope = self as unknown as {
  onmessage: ((event: MessageEvent) => void) | null;
  postMessage(message: unknown): void;
};

scope.onmessage = (event) => {
  const { source, flags, texts } = event.data as { source: string; flags: string; texts: string[] };
  runTrial(source, flags, texts);
  scope.postMessage("done");
};
