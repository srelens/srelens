import { useEffect, useRef, useState } from "react";
import {
  readFreelensExtension,
  type FreelensBootstrap,
  type InstalledExtension,
} from "@srelens/core";
import runtime from "@srelens/lens-compat/runtime?raw";
import styles from "@srelens/lens-compat/styles?raw";
import { useResource } from "../lib/useResource";
import { ErrorNotice } from "./ExtensionResults";

// No host APIs are exposed to the frame. Its only communication channel accepts
// a small read protocol and supplies the cluster/revision from this pinned view.
export function FreelensView({
  plugin,
  context,
}: {
  plugin: InstalledExtension;
  context: string;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [url, setUrl] = useState("");
  const [error, setError] = useState("");
  const bootstrap = useResource(
    () =>
      context
        ? readFreelensExtension<FreelensBootstrap>(
            plugin.manifest.id,
            plugin.revision,
            context,
            { operation: "bootstrap" },
          )
        : Promise.resolve(null),
    [plugin.manifest.id, plugin.revision, context],
  );
  useEffect(() => {
    if (
      bootstrap.status !== "ready" ||
      !bootstrap.data ||
      !bootstrap.data.crds.length
    )
      return;
    let active = true;
    const message = async (event: MessageEvent) => {
      if (
        event.source !== frame.current?.contentWindow ||
        event.origin !== "null"
      )
        return;
      const data = event.data;
      if (data?.type === "ready") {
        frame.current?.contentWindow?.postMessage(
          { type: "initialize", ...bootstrap.data },
          "*",
        );
        return;
      }
      if (data?.type === "error") {
        setError(String(data.error).slice(0, 2000));
        return;
      }
      if (
        data?.type !== "read" ||
        !Number.isSafeInteger(data.sequence) ||
        data.sequence < 0
      )
        return;
      const { operation, group, version, plural, kind } = data.request || {};
      if (
        !["resource", "events"].includes(operation) ||
        (operation === "resource" &&
          ![group, version, plural, kind].every(
            (v) => typeof v === "string" && v.length <= 253,
          ))
      )
        return;
      const target = event.source as Window;
      try {
        const result = await readFreelensExtension(
          plugin.manifest.id,
          plugin.revision,
          context,
          operation === "events"
            ? { operation }
            : { operation, group, version, plural, kind },
        );
        if (active)
          target.postMessage(
            { type: "result", sequence: data.sequence, result },
            "*",
          );
      } catch (e) {
        if (active)
          target.postMessage(
            { type: "result", sequence: data.sequence, error: String(e) },
            "*",
          );
      }
    };
    window.addEventListener("message", message);
    const getTheme = () => {
      const css = getComputedStyle(document.documentElement);
      return ["--ink", "--surface", "--line", "--accent", "--muted"]
        .map(
          (name) =>
            `${name}:${css.getPropertyValue(name === "--line" ? "--rule" : name === "--muted" ? "--ink-muted" : name).trim() || { "--ink": "#29272d", "--surface": "#f8f8fa", "--line": "#dedde3", "--accent": "#a33460", "--muted": "#777" }[name]}`,
        )
        .join(";");
    };
    const theme = getTheme();
    const observer = new MutationObserver(() =>
      frame.current?.contentWindow?.postMessage(
        { type: "theme", style: getTheme() },
        "*",
      ),
    );
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style", "data-theme"],
    });
    const script = `${runtime}\nconst pending=new Map();let sequence=0;let initialized=false;addEventListener('message',event=>{if(event.source!==parent)return;const data=event.data;if(data.type==='theme'){document.documentElement.style.cssText=data.style;}else if(data.type==='initialize'&&!initialized){initialized=true;try{FreelensRuntime.mount({...data,request:request=>new Promise((resolve,reject)=>{const id=sequence++;pending.set(id,{resolve,reject});parent.postMessage({type:'read',sequence:id,request},'*');})});}catch(error){parent.postMessage({type:'error',error:String(error)},'*');}}else if(data.type==='result'){const item=pending.get(data.sequence);if(item){pending.delete(data.sequence);data.error?item.reject(new Error(data.error)):item.resolve(data.result);}}});parent.postMessage({type:'ready'},'*');`;
    const html = `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'"><style>${styles}:root{${theme}}</style><div id="root"></div><script>${script.replace(/<\/script/gi, "<\\/script")}</script>`;
    const blob = URL.createObjectURL(new Blob([html], { type: "text/html" }));
    setError("");
    setUrl(blob);
    return () => {
      active = false;
      observer.disconnect();
      window.removeEventListener("message", message);
      URL.revokeObjectURL(blob);
    };
  }, [
    bootstrap.status,
    bootstrap.data,
    plugin.manifest.id,
    plugin.revision,
    context,
  ]);
  if (!context)
    return (
      <p className="extension-message">
        Choose a cluster before opening the extension.
      </p>
    );
  if (bootstrap.status === "ready" && bootstrap.data?.crds.length === 0)
    return (
      <p className="extension-message">
        This cluster serves no Flux custom resource APIs.
      </p>
    );
  if (bootstrap.status === "error")
    return (
      <ErrorNotice message={bootstrap.error} retry={bootstrap.reload} cluster />
    );
  if (error) return <ErrorNotice message={error} retry={bootstrap.reload} />;
  if (!url)
    return (
      <p role="status" className="extension-message">
        Loading Freelens extension…
      </p>
    );
  return (
    <div className="freelens-view">
      <div className="extension-warning">
        Freelens FluxCD 5.3.1 · Read-only compatibility · Select a resource to
        open its details.
      </div>
      <iframe
        ref={frame}
        title="Freelens FluxCD"
        sandbox="allow-scripts"
        src={url}
      />
    </div>
  );
}
