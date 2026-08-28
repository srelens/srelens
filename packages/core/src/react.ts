/**
 * The parts of the service layer that are React hooks. They are a separate
 * entry point so the main one can be imported by anything — a worker, a test,
 * a future CLI — without pulling React in.
 */
export * from "./lib/access";
export * from "./lib/useNamespaceOptions";
