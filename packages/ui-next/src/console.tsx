import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

/** What a question is delivered to: the dock's own submit handler. */
type Submit = (question: string) => void;

export interface ConsoleValue {
  open: boolean;
  setOpen: (open: boolean) => void;
  /** Open the console and put a question to it, from anywhere in the app. */
  ask: (question: string) => void;
  /** What the console is currently asking about — a cluster, a workload. */
  scope: string;
  setScope: (scope: string) => void;
  /** The dock says it is listening. Returns the way to stop. */
  registerSubmit: (submit: Submit) => () => void;
}

const ConsoleContext = createContext<ConsoleValue | null>(null);

/**
 * The console's state: whether it is open, what it is scoped to, and the way
 * anything on screen can put a question to it.
 *
 * This lives in `ui-next` rather than the kit deliberately. It is state and not
 * presentation, and the kit is components — it may not reach the service layer
 * either, which console state eventually will. `ConsoleDock` stays in the kit
 * and takes what it renders as props, so the two can be developed and tested
 * apart. Same split as `NavIcon`, whose resource map stayed in the app. (#320)
 *
 * `ask` is the part worth reading. The mock opened the dock and then waited ten
 * milliseconds before handing the question over, hoping the dock had mounted
 * and registered in the meantime — so a slow render dropped the question with
 * nothing to show for it. A question asked before anything is listening is held
 * instead, and delivered the moment a dock registers. The mock also never
 * cleared its handler, so a question asked after the dock unmounted called into
 * a component that no longer existed; registering hands back the way to undo it.
 */
export function ConsoleProvider({
  children,
  initialScope = "",
}: {
  children: ReactNode;
  initialScope?: string;
}) {
  const [open, setOpen] = useState(false);
  const [scope, setScope] = useState(initialScope);
  const submit = useRef<Submit | null>(null);
  // Held rather than timed. Anything asked while nothing is listening waits
  // here until something is, which is the difference between a question that
  // arrives late and one that is lost.
  const waiting = useRef<string[]>([]);

  const registerSubmit = useCallback((next: Submit) => {
    submit.current = next;
    const held = waiting.current;
    waiting.current = [];
    for (const question of held) next(question);
    return () => {
      // Only if it is still ours: a dock replaced by another should not have
      // the newcomer's handler torn out from under it on the old one's cleanup.
      if (submit.current === next) submit.current = null;
    };
  }, []);

  const ask = useCallback((question: string) => {
    setOpen(true);
    if (submit.current) submit.current(question);
    else waiting.current.push(question);
  }, []);

  const value = useMemo<ConsoleValue>(
    () => ({ open, setOpen, ask, scope, setScope, registerSubmit }),
    [open, ask, scope, registerSubmit],
  );

  return <ConsoleContext.Provider value={value}>{children}</ConsoleContext.Provider>;
}

/**
 * The console, from anywhere beneath the provider.
 *
 * Throws rather than returning undefined: a hook that quietly hands back
 * nothing turns into "cannot read properties of undefined" in a component three
 * levels from the mistake.
 */
export function useConsole(): ConsoleValue {
  const value = useContext(ConsoleContext);
  if (!value) throw new Error("useConsole must be used inside a ConsoleProvider");
  return value;
}
