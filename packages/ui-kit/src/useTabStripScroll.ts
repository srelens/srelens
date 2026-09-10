import { useEffect, useRef, useState, type RefObject } from "react";

/** Measure the scrolling region, never the fixed controls beside it. */
export function useTabStripScroll(ref: RefObject<HTMLDivElement | null>, tabIds: string) {
  const [edges, setEdges] = useState({ left: false, right: false });
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const repeat = useRef<ReturnType<typeof setInterval> | undefined>(undefined);
  const held = useRef(false);
  const holdingDirection = useRef<-1 | 1 | null>(null);
  function measure() {
    const node = ref.current;
    if (!node) return;
    const left = node.scrollLeft > 1;
    const right = node.scrollWidth - node.clientWidth - node.scrollLeft > 1;
    if ((holdingDirection.current === -1 && !left) || (holdingDirection.current === 1 && !right)) stop();
    setEdges(old => old.left === left && old.right === right ? old : { left, right });
  }
  function stop() {
    clearTimeout(timer.current); clearInterval(repeat.current); holdingDirection.current = null;
  }
  function scroll(direction: -1 | 1, continuous = false) {
    const node = ref.current;
    if (!node) return;
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    node.scrollBy({ left: direction * Math.max(48, node.clientWidth * (continuous ? .2 : .8)), behavior: continuous || reduced ? "auto" : "smooth" });
  }
  function start(direction: -1 | 1) {
    stop(); held.current = false; holdingDirection.current = direction;
    timer.current = setTimeout(() => {
      held.current = true;
      scroll(direction, true);
      repeat.current = setInterval(() => scroll(direction, true), 100);
    }, 350);
  }
  function click(direction: -1 | 1) {
    if (!held.current) scroll(direction);
    held.current = false;
  }
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(node);
    for (const child of node.children) observer?.observe(child);
    node.addEventListener("scroll", measure);
    window.addEventListener("resize", measure);
    window.addEventListener("blur", stop);
    return () => {
      observer?.disconnect(); node.removeEventListener("scroll", measure);
      window.removeEventListener("resize", measure); window.removeEventListener("blur", stop); stop();
    };
  }, [ref, tabIds]);
  return { ...edges, start, stop, click };
}
