/**
 * The srelens service layer: everything between the Rust capabilities and a
 * user interface. React-coupled hooks live behind "@srelens/core/react".
 *
 * One name collides: `ToolStatus` is a string union in `chat` and an interface
 * in `toolbox`. A star export cannot disambiguate them, so the barrel resolves
 * it to chat's explicitly and both consumers take theirs from its own module
 * via "@srelens/core/lib/<name>". Renaming one would be the real fix, but that
 * is an API change, not a move.
 */
export { isTauri, isWeb } from "./transport/platform";
export * from "./transport/transport";
export { csrfHeader } from "./transport/webTransport";
export * from "./lib/actions";
export * from "./lib/addCluster";
export * from "./lib/age";
export * from "./lib/appLog";
export * from "./lib/assistantMarkdown";
export * from "./lib/bulk";
export * from "./lib/chat";
export * from "./lib/chatHistory";
export * from "./lib/clusterLogin";
export * from "./lib/clusters";
export * from "./lib/contextIdentity";
export * from "./lib/controllers";
export * from "./lib/copyKubectl";
export * from "./lib/crds";
export * from "./lib/deepLink";
export * from "./lib/errors";
export * from "./lib/exec";
export * from "./lib/files";
export * from "./lib/forward";
export * from "./lib/helm";
export * from "./lib/kinds";
export * from "./lib/kubectlMapper";
export * from "./lib/llm";
export * from "./lib/logsStream";
export * from "./lib/manifest";
export * from "./lib/manifestEdit";
export * from "./lib/mcp";
export * from "./lib/mcpClients";
export * from "./lib/mcpSecurity";
export * from "./lib/namespaces";
export * from "./lib/network";
export * from "./lib/notify";
export * from "./lib/onboarding";
export * from "./lib/openTabs";
export * from "./lib/overviewSnapshot";
export * from "./lib/paletteActions";
export * from "./lib/podContainers";
export * from "./lib/prompts";
export * from "./lib/rbac";
export * from "./lib/recents";
export * from "./lib/relativeTime";
export * from "./lib/releaseNotes";
export * from "./lib/requestTimeout";
export * from "./lib/resourceNavigation";
export * from "./lib/savedForwards";
export * from "./lib/schema";
export * from "./lib/schemaComplete";
export * from "./lib/serviceAddress";
export * from "./lib/session";
export * from "./lib/settings";
export * from "./lib/settingsStorage";
export * from "./lib/shortcuts";
export * from "./lib/skills";
export * from "./lib/storage";
export * from "./lib/tabs";
export * from "./lib/tabView";
export * from "./lib/terminal";
export * from "./lib/terminalDriver";
export * from "./lib/terminalReconnect";
export * from "./lib/toolbox";
export * from "./lib/uiScale";
export * from "./lib/updateNotifier";
export * from "./lib/updater";
export * from "./lib/watch";
export * from "./lib/webClusters";
export * from "./lib/webKubeconfigs";
export * from "./lib/workloads";

// Disambiguates the star exports above; see the note in the header.
export type { ToolStatus } from "./lib/chat";
