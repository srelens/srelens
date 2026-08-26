import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

/**
 * A part of the window that owns the layers opened inside it.
 *
 * `container` is the node those layers mount into, and `hold` is how a layer
 * says it is open — the surface needs the count to make its own content
 * unreachable while a layer covers it.
 */
export interface PortalScope {
  /** Where layers in this scope mount. Undefined until the node exists. */
  container: HTMLElement | undefined;
  /**
   * Whether this surface is the one on screen. A surface that hides rather
   * than unmounts keeps its layers mounted with it, and a layer that cannot
   * tell it is off screen will answer key presses meant for the surface the
   * reader is actually looking at.
   */
  visible: boolean;
  /** Registers one open layer. Call the returned function to release it. */
  hold: () => () => void;
}

/**
 * Undefined by default, and that default is load-bearing rather than an
 * oversight: a dialog rendered with no surface around it — the gallery, the
 * frozen classic app, most of this kit's own tests — must behave exactly as it
 * did before this existed. `undefined` is what Radix's `container` prop wants
 * in order to fall back to `document.body`.
 */
const ScopeContext = createContext<PortalScope | undefined>(undefined);

/**
 * Puts everything below it inside one surface's scope.
 *
 * Wraps both the surface's content and the node its layers mount into, so
 * `usePortalHost` can hand one `scope` to one provider — see that hook for why
 * the two have to be siblings.
 */
export function PortalScopeProvider({ scope, children }: { scope: PortalScope; children: ReactNode }) {
  return <ScopeContext.Provider value={scope}>{children}</ScopeContext.Provider>;
}

/**
 * Where a portalled component should mount: the surrounding surface's node, or
 * `undefined` when there is no surface, which sends it to `document.body`.
 *
 * This is the whole API a menu, a popover or a tooltip needs. A layer that also
 * covers the surface wants {@link useOpenLayer} instead.
 */
export function usePortalContainer(): HTMLElement | undefined {
  return useContext(ScopeContext)?.container;
}

/**
 * Whether there is a surface around this component at all.
 *
 * The other half of what {@link usePortalContainer} answers, for the component
 * that needs both: `container` says where to render, and this says whether
 * there is a surface to be modal *within*. They are not the same question and
 * they differ for one render — see {@link useOpenLayer}, which hands a layer
 * both — so a component that has a use for the second must ask for it rather
 * than infer it from the first.
 */
export function usePortalScoped(): boolean {
  return useContext(ScopeContext) !== undefined;
}

/**
 * For a layer that covers the surface it belongs to, and must be counted while
 * it does: a dialog, not a tooltip.
 *
 * The registration lasts as long as the caller is mounted, which is the right
 * lifetime for the two dialogs in this kit — both are mounted only while open.
 * A component that stays mounted across open and closed would hold the surface
 * covered the whole time and must not use this hook.
 *
 * `scoped` is a separate answer from `container`, deliberately. A layer needs
 * to know whether there is a surface to be modal *within*, and that is not the
 * same as whether the node exists yet: they differ for the render between a
 * surface mounting and its ref firing, and a layer reading only the container
 * would treat that render as "no surface at all" and set up as a document-wide
 * modal for it.
 *
 * `showing` is true outside any surface, because a layer with no surface around
 * it is never the one hidden behind another.
 */
export function useOpenLayer(): { container: HTMLElement | undefined; scoped: boolean; showing: boolean } {
  const scope = useContext(ScopeContext);
  // The hold, not the scope: the scope's identity changes when its node
  // arrives, and depending on it would release and re-take the registration
  // for nothing. `hold` is stable for the surface's lifetime.
  const hold = scope?.hold;
  useEffect(() => hold?.(), [hold]);
  return { container: scope?.container, scoped: scope !== undefined, showing: scope?.visible ?? true };
}

/**
 * The state a surface needs to host its own layers.
 *
 * `ref` goes on the node the layers mount into, and `layered` is true while at
 * least one of them is open — which is the surface's cue to mark its content
 * `inert`. That is why the two must be *siblings*: an element cannot be marked
 * inert and also be the ancestor of the thing that is meant to stay reachable.
 *
 * The node is held in state rather than a ref because the container has to
 * reach the layers through a render; a ref would be filled after the render
 * that needed it and nothing would re-read it.
 *
 * `visible` is what the surface would put on its own `hidden` attribute,
 * inverted. It defaults to true for a surface that is always on screen.
 */
export function usePortalHost(visible = true): {
  /** Attach to the node layers mount into. It must be a sibling of the content. */
  ref: (node: HTMLElement | null) => void;
  /** True while a layer is open: mark the content `inert`. */
  layered: boolean;
  /** Hand to {@link PortalScopeProvider}, around both the content and the node. */
  scope: PortalScope;
} {
  const [container, setContainer] = useState<HTMLElement | undefined>(undefined);
  const [layers, setLayers] = useState(0);

  const hold = useCallback(() => {
    setLayers((n) => n + 1);
    // Guarded because a release called twice would uncover the surface while a
    // second layer is still open. React's own effect cleanup does not do that,
    // but this function is handed out and the count is shared.
    let released = false;
    return () => {
      if (released) return;
      released = true;
      setLayers((n) => n - 1);
    };
  }, []);

  const ref = useCallback((node: HTMLElement | null) => {
    setContainer(node ?? undefined);
  }, []);

  const scope = useMemo(() => ({ container, visible, hold }), [container, visible, hold]);
  return { ref, layered: layers > 0, scope };
}
