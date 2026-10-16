import { answer, type RegexItem } from "./regexMatch";

// Typed by hand, not with the `webworker` lib: that lib redeclares globals the
// DOM lib already has, so it cannot share a tsconfig with the rest of this package.
const scope = self as unknown as {
  onmessage: ((event: MessageEvent) => void) | null;
  postMessage(message: unknown): void;
};

scope.onmessage = (event) => {
  const { id, source, flags, items } = event.data as { id: number; source: string; flags: string; items: RegexItem[] };
  // First, so the host times the pattern and not this worker's start-up (see `createRegexOracle`).
  scope.postMessage({ id, started: true });
  scope.postMessage({ id, answers: answer(source, flags, items) });
};
