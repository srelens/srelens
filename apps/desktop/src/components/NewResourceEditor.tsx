import React, { useState } from "react";
import { FilePlus2 } from "lucide-react";
import { Combobox } from "../ui";
import { ManifestEditor } from "./ManifestEditor";

/** Starter manifests for common kinds, namespaced where relevant. */
const TEMPLATES: Record<string, (ns: string) => string> = {
  Blank: () => "",
  Deployment: (ns) =>
    `apiVersion: apps/v1
kind: Deployment
metadata:
  name: my-app
  namespace: ${ns}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: my-app
  template:
    metadata:
      labels:
        app: my-app
    spec:
      containers:
        - name: app
          image: nginx:1.27
          ports:
            - containerPort: 80
`,
  Service: (ns) =>
    `apiVersion: v1
kind: Service
metadata:
  name: my-app
  namespace: ${ns}
spec:
  selector:
    app: my-app
  ports:
    - port: 80
      targetPort: 80
`,
  ConfigMap: (ns) =>
    `apiVersion: v1
kind: ConfigMap
metadata:
  name: my-config
  namespace: ${ns}
data:
  key: value
`,
  Secret: (ns) =>
    `apiVersion: v1
kind: Secret
metadata:
  name: my-secret
  namespace: ${ns}
type: Opaque
stringData:
  key: value
`,
  Ingress: (ns) =>
    `apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: my-ingress
  namespace: ${ns}
spec:
  rules:
    - host: example.com
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: my-app
                port:
                  number: 80
`,
  Namespace: () =>
    `apiVersion: v1
kind: Namespace
metadata:
  name: my-namespace
`,
};

const TEMPLATE_ORDER = ["Blank", "Deployment", "Service", "ConfigMap", "Secret", "Ingress", "Namespace"];

/**
 * A full-tab "new resource" editor: pick a starter template, edit YAML with
 * k8s syntax highlighting, and apply (server-side apply, which creates when the
 * object doesn't exist). Stays open after applying so you can create several.
 */
export function NewResourceEditor({
  context,
  namespace = "default",
  initialKind,
  onCreated,
}: {
  context: string;
  namespace?: string;
  /** k8s Kind (e.g. "Deployment") to preselect a template. */
  initialKind?: string;
  onCreated?: () => void;
}) {
  const ns = namespace || "default";
  const startTemplate = initialKind && TEMPLATES[initialKind] ? initialKind : "Deployment";
  const [template, setTemplate] = useState(startTemplate);
  const [yaml, setYaml] = useState(() => TEMPLATES[startTemplate](ns));

  function pickTemplate(t: string) {
    setTemplate(t);
    setYaml(TEMPLATES[t](ns));
  }

  return (
    <ManifestEditor
      context={context}
      namespace={ns}
      yaml={yaml}
      onYamlChange={setYaml}
      ariaLabel="New resource YAML"
      fill
      headerLabel="New resource"
      applyLabel="Create"
      applyingLabel="Creating…"
      applyIcon={<FilePlus2 data-icon="inline-start" />}
      onApplied={onCreated}
      headerExtras={
        <>
          <span className="mx-1 text-xs text-muted-foreground">Template</span>
          <Combobox
            value={template}
            onValueChange={pickTemplate}
            options={TEMPLATE_ORDER.map((t) => ({ value: t }))}
            ariaLabel="Template"
            searchPlaceholder="Search templates…"
            className="min-w-40"
          />
        </>
      }
    />
  );
}
