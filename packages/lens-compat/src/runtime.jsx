// Runs only inside the opaque-origin compatibility iframe. Never import host
// transport, Tauri, credentials, filesystem or Node execution into this bundle.
import React, { useEffect, useState } from "react";
import ReactDom from "react-dom";
import * as ReactJsxRuntime from "react/jsx-runtime";
import * as Mobx from "mobx";
import * as MobxReact from "mobx-react";
import * as ReactRouterDom from "react-router-dom";
import { sha256 } from "@noble/hashes/sha2.js";

export function mount({
  source,
  crds,
  namespaces,
  page = "dashboard",
  request,
}) {
  const stores = new Map();
  let extension;
  let navigate = () => {};
  let showDetails = () => {};
  let report = () => {};
  const read = async (args) => {
    try {
      return await request(args);
    } catch (error) {
      report(String(error));
      throw error;
    }
  };
  const namespaceState = Mobx.observable({ selected: "" });
  const age = (value) => {
    const seconds = Math.max(
      0,
      (Date.now() - new Date(value).getTime()) / 1000,
    );
    if (!Number.isFinite(seconds)) return "—";
    for (const [unit, size] of [
      ["y", 31536000],
      ["d", 86400],
      ["h", 3600],
      ["m", 60],
      ["s", 1],
    ])
      if (seconds >= size) return `${Math.floor(seconds / size)}${unit}`;
    return "0s";
  };
  class KubeObject {
    constructor(data = {}) {
      Object.assign(this, data);
      this.metadata ||= {};
      this.spec ||= {};
    }
    getName() {
      return this.metadata.name || "";
    }
    getNs() {
      return this.metadata.namespace || "";
    }
    getId() {
      return (
        this.metadata.uid ||
        `${this.apiVersion}/${this.kind}/${this.getNs()}/${this.getName()}`
      );
    }
    getSearchFields() {
      return [this.getName(), this.getNs()];
    }
    getCreationTimestamp() {
      return new Date(this.metadata.creationTimestamp || 0).getTime();
    }
    getAge() {
      return age(this.metadata.creationTimestamp);
    }
    getLabels() {
      return Object.entries(this.metadata.labels || {}).map(
        ([k, v]) => `${k}=${v}`,
      );
    }
    getAnnotations() {
      return Object.entries(this.metadata.annotations || {}).map(
        ([k, v]) => `${k}=${v}`,
      );
    }
    getOwnerRefs() {
      return this.metadata.ownerReferences || [];
    }
    getResourceVersion() {
      return this.metadata.resourceVersion;
    }
    getSelfLink() {
      return `${this.constructor.apiBase}/namespaces/${this.getNs()}/${this.getName()}`;
    }
    static getApi() {
      return this.getStore().api;
    }
    static getStore() {
      const [group, version] = (this.crd?.apiVersions?.[0] || "").split("/");
      const plural = this.crd?.plural;
      const key = `${group}/${version}/${plural}/${this.kind}`;
      if (stores.has(key)) return stores.get(key);
      const crd = crds.find(
        (c) =>
          c.group === group &&
          c.plural === plural &&
          c.kind === this.kind &&
          c.versions.includes(version),
      );
      if (!crd) throw new Error(`API not registered: ${this.apiBase}`);
      const store = new KubeObjectStore(
        { group, version, plural, kind: this.kind },
        this,
      );
      stores.set(key, store);
      return store;
    }
  }
  class KubeObjectStore {
    items = [];
    isLoading = false;
    isLoaded = false;
    constructor(resource, Type = KubeObject) {
      this.resource = resource;
      this.Type = Type;
      this.api = {
        kind: resource.kind,
        apiGroup: resource.group,
        get: ({ name, namespace }) => this.getByName(name, namespace),
        patch: async () => {
          throw new Error(
            "Cluster writes are not supported by this read-only compatibility runtime",
          );
        },
        formatUrlForNotListing: ({ name, namespace }) =>
          `${resource.group}/${resource.plural}/${namespace}/${name}`,
      };
      Mobx.makeObservable(this, {
        items: Mobx.observable.shallow,
        isLoading: Mobx.observable,
        isLoaded: Mobx.observable,
        contextItems: Mobx.computed,
      });
    }
    get contextItems() {
      return this.items.filter(
        (o) =>
          !namespaceState.selected || o.getNs() === namespaceState.selected,
      );
    }
    getByName(name, namespace) {
      return this.items.find(
        (o) => o.getName() === name && (!namespace || o.getNs() === namespace),
      );
    }
    async loadAll() {
      if (this.loading) return this.loading;
      Mobx.runInAction(() => {
        this.isLoading = true;
      });
      this.loading = read({ operation: "resource", ...this.resource })
        .then((result) => {
          const items = (result.objects || []).map((o) => new this.Type(o));
          Mobx.runInAction(() => {
            this.items = items;
            this.isLoaded = true;
          });
          return items;
        })
        .finally(() => {
          this.loading = null;
          Mobx.runInAction(() => {
            this.isLoading = false;
          });
        });
      return this.loading;
    }
    subscribe() {
      const timer = setInterval(
        () => void this.loadAll().catch(() => {}),
        30000,
      );
      return () => clearInterval(timer);
    }
  }
  const namespaceStore = {
    items: namespaces.map((name) => new KubeObject({ metadata: { name } })),
    loadAll: async () => namespaceStore.items,
    subscribe: () => () => {},
    getContextNamespaces: () =>
      namespaceState.selected ? [namespaceState.selected] : namespaces,
  };
  const crdStore = {
    loadAll: async () =>
      crds.map(
        (c) =>
          new KubeObject({
            metadata: { name: `${c.plural}.${c.group}` },
            spec: {
              group: c.group,
              names: {
                kind: c.kind,
                plural: c.plural,
                singular: c.plural.replace(/ies$/, "y").replace(/s$/, ""),
              },
              versions: c.versions.map((name) => ({ name, served: true })),
            },
          }),
      ),
  };
  const eventStore = new KubeObjectStore({ kind: "Event" });
  eventStore.loadAll = async () => {
    const result = await read({ operation: "events" });
    const items = (result.events || []).map(
      (e) =>
        new KubeObject({
          metadata: {
            name: e.name,
            namespace: e.namespace,
            creationTimestamp: e.created,
          },
          type: e.type,
          message: e.message,
          count: e.count,
          source: { component: e.source },
          involvedObject: {
            apiVersion: e.objectApiVersion,
            kind: e.objectKind,
            name: e.objectName,
            namespace: e.namespace,
          },
          lastTimestamp: e.created,
        }),
    );
    Mobx.runInAction(() => {
      eventStore.items = items;
      eventStore.isLoaded = true;
    });
    return items;
  };
  const WithTooltip = ({ children, tooltip }) => (
    <span title={typeof tooltip === "string" ? tooltip : undefined}>
      {children}
    </span>
  );
  let nextPickerId = 0;
  const NamespaceFilter = MobxReact.observer(() => {
    const [search, setSearch] = useState(namespaceState.selected),
      [open, setOpen] = useState(false),
      [id] = useState(() => `namespaces-${nextPickerId++}`);
    useEffect(
      () => setSearch(namespaceState.selected),
      [namespaceState.selected],
    );
    const choices = ["", ...namespaces].filter(
      (n) => !search || n.toLowerCase().includes(search.toLowerCase()),
    );
    const choose = (value) => {
      Mobx.runInAction(() => {
        namespaceState.selected = value;
      });
      setSearch(value);
      setOpen(false);
    };
    return (
      <div className="namespace-picker">
        <input
          role="combobox"
          aria-label="Filter namespace"
          aria-expanded={open}
          aria-controls={id}
          aria-autocomplete="list"
          placeholder="All namespaces"
          value={search}
          onFocus={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onChange={(e) => {
            setSearch(e.target.value);
            setOpen(true);
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") setOpen(false);
            if (e.key === "Enter" && choices.length) {
              e.preventDefault();
              choose(choices[0]);
            }
          }}
        />
        {open && (
          <div id={id} role="listbox">
            {choices.map((n) => (
              <button
                key={n}
                role="option"
                aria-selected={namespaceState.selected === n}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => choose(n)}
              >
                {n || "All namespaces"}
              </button>
            ))}
          </div>
        )}
      </div>
    );
  });
  const ObjectLink = ({ object, name, namespace, kind, children }) => (
    <button
      className="link"
      onClick={async () => {
        if (object instanceof KubeObject) return showDetails(object);
        const match = [...stores.values()].find(
          (s) => s.resource.kind === (kind || object?.kind),
        );
        if (!match)
          return report(
            "The related resource API is not available in this read-only extension view.",
          );
        await match.loadAll().catch(() => {});
        const found = match.getByName(
          name || object?.name,
          namespace || object?.namespace,
        );
        if (found) showDetails(found);
        else
          report(
            "The related resource could not be found in the loaded namespace.",
          );
      }}
    >
      {children || name || object?.name || "View resource"}
    </button>
  );
  const ListLayout = MobxReact.observer((props) => {
    const {
      store,
      renderTableHeader = [],
      renderTableContents,
      renderHeaderTitle,
    } = props;
    const [search, setSearch] = useState("");
    useEffect(() => {
      void store.loadAll().catch(() => {});
      return store.subscribe();
    }, [store]);
    const items = (
      props.getItems ? props.getItems() : store.contextItems
    ).filter(
      (o) =>
        !search ||
        JSON.stringify(o).toLowerCase().includes(search.toLowerCase()),
    );
    return (
      <section className="resource-list">
        <header>
          <strong>{renderHeaderTitle}</strong>
          <NamespaceFilter />
          <input
            aria-label="Search resources"
            placeholder="Search resources…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <button onClick={() => void store.loadAll().catch(() => {})}>
            Refresh
          </button>
        </header>
        {store.isLoading && !store.isLoaded ? (
          <p role="status">Loading resources…</p>
        ) : (
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  {renderTableHeader.map((h, i) => (
                    <th key={i}>{h.title}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {items.map((o) => (
                  <tr key={o.getId()}>
                    {renderTableContents(o).map((cell, i) => (
                      <td key={i}>
                        {i === 0 ? (
                          <button
                            className="link"
                            data-resource-name={o.getName()}
                            onClick={() => showDetails(o)}
                          >
                            {cell}
                          </button>
                        ) : (
                          cell
                        )}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
            {!items.length && <p>No matching resources.</p>}
          </div>
        )}
      </section>
    );
  });
  function PieChart({ data }) {
    const values = data.datasets[0].data,
      colors = data.datasets[0].backgroundColor,
      total = values.reduce((a, b) => a + b, 0);
    let start = 0;
    const stops = values.map((v, i) => {
      const end = start + (total ? (v / total) * 100 : 0);
      const stop = `${colors[i]} ${start}% ${end}%`;
      start = end;
      return stop;
    });
    return (
      <div className="chart">
        <div
          role="img"
          aria-label={data.labels.join(", ")}
          className="donut"
          style={{
            background: total ? `conic-gradient(${stops.join(",")})` : "#777",
          }}
        />
        <ul>
          {data.labels.map((label, i) => (
            <li key={i}>
              <i style={{ background: colors[i] }} />
              {label}
            </li>
          ))}
        </ul>
      </div>
    );
  }
  const components = {
    KubeObjectListLayout: ListLayout,
    NamespaceSelectFilter: NamespaceFilter,
    NamespaceSelectBadge: ({ namespace }) => <span>{namespace}</span>,
    TabLayout: ({ children }) => <div>{children}</div>,
    WithTooltip,
    Badge: ({ label, children, className }) => (
      <span className={className}>{label ?? children}</span>
    ),
    BadgeBoolean: ({ value }) => <span>{value ? "Yes" : "No"}</span>,
    DrawerItem: ({ name, children, hidden }) =>
      hidden ? null : (
        <div className="drawer-item">
          <dt>{name}</dt>
          <dd>{children ?? "—"}</dd>
        </div>
      ),
    DrawerTitle: ({ title, children }) => <h3>{title || children}</h3>,
    DrawerItemLabels: ({ labels }) => (
      <span>
        {Array.isArray(labels) ? labels.join(", ") : JSON.stringify(labels)}
      </span>
    ),
    Icon: ({ tooltip, material }) => (
      <span title={typeof tooltip === "string" ? tooltip : undefined}>
        {material === "check" ? "✓" : material === "close" ? "×" : ""}
      </span>
    ),
    LinkToObject: ObjectLink,
    // These links can target credentials. Keep them inert and explicit; the
    // compatibility broker deliberately has no core-resource/Secret read grant.
    ...Object.fromEntries(
      [
        "LinkToSecret",
        "LinkToConfigMap",
        "LinkToServiceAccount",
        "LinkToStorageClass",
        "LinkToNamespace",
      ].map((k) => [
        k,
        ({ name, children }) => (
          <span title="Open related core resources in the host resource browser">
            {children || name}
          </span>
        ),
      ]),
    ),
    MonacoEditor: ({ value }) => <pre>{value}</pre>,
    KubeObjectAge: ({ object }) => <span>{object.getAge()}</span>,
    LocaleDate: ({ date }) => (
      <span>{date ? new Date(date).toLocaleString() : "—"}</span>
    ),
    ReactiveDuration: ({ timestamp }) => <span>{age(timestamp)}</span>,
    DurationAbsoluteTimestamp: ({ timestamp }) => (
      <span>{timestamp || "—"}</span>
    ),
    MaybeLink: ({ children }) => <span>{children}</span>,
    MenuItem: ({ children }) => (
      <button disabled title="Read-only compatibility runtime">
        {children}
      </button>
    ),
    Table: ({ children }) => (
      <table>
        <tbody>{children}</tbody>
      </table>
    ),
    TableHead: ({ children }) => <tr>{children}</tr>,
    TableRow: ({ children }) => <tr>{children}</tr>,
    TableCell: ({ children }) => <td>{children}</td>,
    PieChart,
  };
  class LensExtension {
    name = "FluxCD";
    getPageUrl({ pageId }) {
      return pageId;
    }
  }
  const cssNames = (...args) =>
    args
      .flatMap((a) =>
        typeof a === "string"
          ? a
          : a && typeof a === "object"
            ? Object.keys(a).filter((k) => a[k])
            : [],
      )
      .join(" ");
  const global = {
    React,
    ReactDom,
    ReactJsxRuntime,
    Mobx,
    MobxReact,
    ReactRouterDom,
    LensExtensions: {
      Main: { LensExtension },
      Common: {
        Util: { cssNames, stopPropagation: (e) => e.stopPropagation() },
        logger: { error: () => {}, debug: () => {} },
      },
      Renderer: {
        LensExtension,
        Component: components,
        K8sApi: {
          LensExtensionKubeObject: KubeObject,
          KubeObject,
          KubeApi: class {},
          KubeObjectStore,
          namespaceStore,
          crdStore,
          eventStore,
          apiManager: { lookupApiLink: (object) => JSON.stringify(object) },
        },
        Navigation: {
          navigate: (target) =>
            navigate(typeof target === "string" ? target : target.pathname),
          getDetailsUrl: (link) => `#details=${encodeURIComponent(link)}`,
        },
      },
    },
  };
  const crypto = {
    createHash(algorithm) {
      if (algorithm !== "sha256")
        throw new Error(`Unsupported hash: ${algorithm}`);
      let data = "";
      return {
        update(text) {
          data += text;
          return this;
        },
        digest(encoding) {
          if (encoding !== "hex") throw new Error("Only hex digests supported");
          return Array.from(sha256(new TextEncoder().encode(data)), (v) =>
            v.toString(16).padStart(2, "0"),
          ).join("");
        },
      };
    },
  };
  const module = { exports: {} };
  new Function("global", "module", "exports", "require", source)(
    global,
    module,
    module.exports,
    (name) => {
      if (name === "crypto" || name === "node:crypto") return crypto;
      throw new Error(`Unsupported Freelens module: ${name}`);
    },
  );
  extension = new module.exports.default();
  const menus = [
    ...new Map(
      extension.clusterPageMenus.map((menu) => [menu.id, menu]),
    ).values(),
  ];
  class Boundary extends React.Component {
    state = { error: null };
    static getDerivedStateFromError(error) {
      return { error: String(error) };
    }
    render() {
      return this.state.error ? (
        <p role="alert">Extension rendering failed: {this.state.error}</p>
      ) : (
        this.props.children
      );
    }
  }
  function App() {
    const [current, setCurrent] = useState(page),
      [object, setObject] = useState(null),
      [error, setError] = useState(""),
      [tab, setTab] = useState("details");
    navigate = (id) => {
      setCurrent(id.replace(/^\//, ""));
      setObject(null);
    };
    showDetails = (o) => {
      setObject(o);
      setTab("details");
    };
    report = setError;
    useEffect(() => {
      const escape = (e) => {
        if (e.key === "Escape") setObject(null);
      };
      document.addEventListener("keydown", escape);
      return () => document.removeEventListener("keydown", escape);
    }, []);
    useEffect(() => {
      if (!object) return;
      const previous = document.activeElement,
        panel = document.querySelector('[role="dialog"]');
      panel?.querySelector("button")?.focus();
      const trap = (event) => {
        if (event.key !== "Tab" || !panel) return;
        const focusable = [
          ...panel.querySelectorAll(
            'button:not(:disabled),a[href],input,textarea,[tabindex="0"]',
          ),
        ];
        const first = focusable[0],
          last = focusable[focusable.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first?.focus();
        }
      };
      document.addEventListener("keydown", trap);
      return () => {
        document.removeEventListener("keydown", trap);
        if (previous?.isConnected) previous.focus();
      };
    }, [object]);
    const Page = extension.clusterPages.find((p) => p.id === current)
      ?.components?.Page;
    const details = object
      ? extension.kubeObjectDetailItems
          .filter(
            (d) =>
              d.kind === object.kind &&
              d.apiVersions.includes(object.apiVersion),
          )
          .filter(
            (d, i, all) =>
              all.findIndex(
                (other) =>
                  String(other.components.Details) ===
                  String(d.components.Details),
              ) === i,
          )
      : [];
    return (
      <ReactRouterDom.MemoryRouter>
        <nav aria-label="FluxCD pages">
          {menus
            .filter((m) => m.parentId === "fluxcd")
            .map((m) => {
              const children = menus.filter((child) => child.parentId === m.id);
              return children.length ? (
                <details className="page-menu" key={m.id}>
                  <summary>{m.title}</summary>
                  <div>
                    {children.map((child) => (
                      <button
                        key={child.id}
                        aria-current={
                          current === child.target?.pageId ? "page" : undefined
                        }
                        onClick={(event) => {
                          navigate(child.target.pageId);
                          event.currentTarget.closest("details").open = false;
                        }}
                      >
                        {child.title}
                      </button>
                    ))}
                  </div>
                </details>
              ) : (
                <button
                  key={m.id}
                  aria-current={
                    current === m.target?.pageId ? "page" : undefined
                  }
                  onClick={() => navigate(m.target.pageId)}
                >
                  {m.title}
                </button>
              );
            })}
        </nav>
        {error && (
          <div role="alert">
            {error}
            <button onClick={() => setError("")}>Dismiss</button>
            <button
              onClick={() => {
                setError("");
                for (const store of stores.values())
                  void store.loadAll().catch(() => {});
              }}
            >
              Retry
            </button>
          </div>
        )}
        <Boundary key={current}>
          {Page ? (
            <Page extension={extension} />
          ) : (
            <p>Extension page unavailable: {current}</p>
          )}
        </Boundary>
        {object && (
          <aside
            className="details"
            role="dialog"
            aria-modal="true"
            aria-label={`${object.kind} ${object.getName()}`}
          >
            <header>
              <strong>
                {object.kind}: {object.getName()}
              </strong>
              <button
                aria-label="Close details"
                onClick={() => setObject(null)}
              >
                Close
              </button>
            </header>
            <nav>
              <button onClick={() => setTab("details")}>Details</button>
              <button onClick={() => setTab("object")}>Object</button>
            </nav>
            <div className="detail-content">
              <p>{object.getNs()}</p>
              {object.spec?.url && (
                <div className="drawer-item">
                  <dt>URL</dt>
                  <dd>{object.spec.url}</dd>
                </div>
              )}
              <Boundary key={object.getId() + tab}>
                {tab === "object" ? (
                  <pre>{JSON.stringify(object, null, 2)}</pre>
                ) : details.length ? (
                  details.map((d, i) => (
                    <d.components.Details
                      key={i}
                      object={object}
                      extension={extension}
                    />
                  ))
                ) : (
                  <pre>{JSON.stringify(object, null, 2)}</pre>
                )}
              </Boundary>
            </div>
          </aside>
        )}
      </ReactRouterDom.MemoryRouter>
    );
  }
  ReactDom.render(<App />, document.getElementById("root"));
}
