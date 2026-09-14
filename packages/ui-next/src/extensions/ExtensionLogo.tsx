import type { IconComponent } from "@srelens/ui-kit";
import { Image, Images, Shield, Webhook } from "lucide-react";
import { Icons } from "../lib/icons";

const logos: Record<string, string> = {
  "org.srelens.flux": new URL("./logos/flux.svg", import.meta.url).href,
  "org.srelens.argocd": new URL("./logos/argo.svg", import.meta.url).href,
};
/** Decorative: the adjacent extension name supplies the accessible label. */
export function ExtensionLogo({ id, name, size = 20, className }: {
  id: string; name: string; size?: number; className?: string;
}) {
  const source = Object.hasOwn(logos, id) ? logos[id] : undefined;
  const initials = name.trim().split(/\s+/).map(word => Array.from(word)[0] ?? "").slice(0, 2).join("").toUpperCase() || "EX";
  return <svg width={size} height={size} viewBox="0 0 24 24" className={`extension-logo ${className ?? ""}`} aria-hidden="true" focusable="false" data-extension-logo={id}>
    {source ? <image href={source} width="24" height="24" preserveAspectRatio="xMidYMid meet" /> : <>
      <rect x="1" y="1" width="22" height="22" rx="5" fill="currentColor" opacity="0.12" />
      <text x="12" y="12" dy=".35em" textAnchor="middle" fontSize="10" fontWeight="600" fill="currentColor">{initials}</text>
    </>}
  </svg>;
}
export function extensionLogoIcon(id: string, name: string): IconComponent {
  return function Logo(props) { return <ExtensionLogo {...props} size={Math.max(props.size ?? 16, 16)} id={id} name={name} />; };
}
const pageIcons: Record<string, IconComponent> = {
  overview: Icons.overview,
  applications: Icons.deployments,
  kustomizations: Icons.workloads,
  helm: Icons.helmreleases,
  helmreleases: Icons.helmreleases,
  helmrepositories: Icons.storage,
  helmcharts: Icons.pods,
  sources: Icons.endpointslices,
  gitrepositories: Icons.endpointslices,
  buckets: Icons.statefulsets,
  ocirepositories: Icons.storage,
  imageautomation: Images,
  imagerepositories: Image,
  imagepolicies: Shield,
  imageupdateautomations: Icons.refresh,
  notifications: Icons.events,
  alerts: Icons.warn,
  providers: Icons.endpoints,
  receivers: Webhook,
};
export function extensionPageIcon(title: string): IconComponent {
  const key = title.toLowerCase().replace(/[^a-z]/g, "");
  return Object.hasOwn(pageIcons, key) ? pageIcons[key] : Icons.config;
}
