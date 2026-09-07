import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { isApplePlatform } from "@srelens/core";

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
  /**
   * What the dock needs to render itself, carried here so it can be mounted
   * ANYWHERE inside this provider rather than only where props reach.
   *
   * `/agent` mounts it at the foot of its own main column, so that screen's
   * rail is a full-height sibling and uses the bottom of the window instead of
   * stopping short of it. Threading two props down through a screen to reach
   * one component is how a shell grows a prop nobody can trace.
   */
  apple: boolean;
  onToggleTheme: () => void;
  /**
   * The draft: what is typed at the prompt, and the images attached to it.
   *
   * Here rather than in the dock, because the dock has TWO mount points — the
   * window's bottom edge, and `/agent`'s own main column — and switching
   * between them unmounts one instance and mounts the other. A draft held in
   * component state was silently lost on that switch, along with any pasted
   * screenshots. The provider outlives both.
   */
  draft: string;
  /** Takes an updater as well as a value: the composer restores a refused
   *  question only if the field is still empty, which it cannot know without
   *  reading the current draft at the moment it writes. */
  setDraft: (next: string | ((held: string) => string)) => void;
  images: string[];
  setImages: (next: string[] | ((held: string[]) => string[])) => void;
  /**
   * How many pasted or picked images are still being read.
   *
   * With the draft, and for the same reason. `FileReader` is asynchronous and
   * Enter is not, so a submission has to wait for a read that belongs to it —
   * and while this counter was component state it reset to zero on the remount
   * a tab switch causes, while the read still in flight went on to append to
   * the provider's `images`. The replacement composer then allowed Enter, sent
   * the question without the screenshot, and the read attached it to the NEXT
   * question: exactly the defect the counter exists to prevent, reappearing
   * across the remount.
   */
  reading: number;
  setReading: (next: number | ((n: number) => number)) => void;
  /**
   * A pasted or picked image that could not be read, and a question refused
   * because there was no cluster to ask about.
   *
   * Here for the same reason as everything above: this is the state of the
   * question being COMPOSED, and the dock has two mount points, so a tab switch
   * unmounts the instance holding it. `attachError` in particular arrives late —
   * a read that fails after the switch set state on the unmounted instance while
   * the provider-level counter still decremented, so the replacement composer
   * showed neither an attachment nor a reason it was missing.
   *
   * Everything belonging to the composition of a question lives here. Splitting
   * it has now produced the same defect three times: `images` without
   * `reading`, and `reading` without this.
   */
  attachError: unknown;
  setAttachError: (e: unknown) => void;
  /**
   * Puts a consumed attachment failure back, but only if nothing has happened
   * since.
   *
   * A method rather than an updater on `setAttachError`, because the state is
   * `unknown` and `unknown | ((held: unknown) => unknown)` collapses back to
   * `unknown` — there is no way to type the updater form. The rule lives here
   * instead, where it can read the current value.
   */
  restoreAttachError: (e: unknown) => void;
  noCluster: boolean;
  setNoCluster: (v: boolean) => void;
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
  onToggleTheme = () => {},
}: {
  children: ReactNode;
  initialScope?: string;
  /** Passed through to the dock, wherever it is mounted. */
  onToggleTheme?: () => void;
}) {
  // Derived here rather than taken as a prop: it is platform detection with no
  // dependency on anything above, and the one consumer is inside this tree.
  const apple = useMemo(() => isApplePlatform(), []);
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [images, setImages] = useState<string[]>([]);
  const [reading, setReading] = useState(0);
  const [attachError, setAttachError] = useState<unknown>(null);
  const restoreAttachError = useCallback((e: unknown) => {
    setAttachError((held: unknown) => (held === null ? e : held));
  }, []);
  const [noCluster, setNoCluster] = useState(false);
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
    () => ({
      open,
      setOpen,
      ask,
      scope,
      setScope,
      registerSubmit,
      apple,
      onToggleTheme,
      draft,
      setDraft,
      images,
      setImages,
      reading,
      setReading,
      attachError,
      setAttachError,
      restoreAttachError,
      noCluster,
      setNoCluster,
    }),
    [open, ask, scope, registerSubmit, apple, onToggleTheme, draft, images, reading, attachError, restoreAttachError, noCluster],
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
