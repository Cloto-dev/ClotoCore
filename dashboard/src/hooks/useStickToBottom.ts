import { type RefObject, useCallback, useEffect, useRef } from 'react';

/** How close to the bottom still counts as "the user is reading the newest turn". */
const BOTTOM_THRESHOLD_PX = 50;

/**
 * Keep a scroll container pinned to its newest content, until the user scrolls
 * away — then leave them where they are, and re-pin when they come back down.
 *
 * Deliberately observer-driven rather than dependency-driven. The chat console
 * used to scroll from a `useEffect` whose dependency array listed the message
 * state as change triggers; a `useExhaustiveDependencies` autofix stripped those
 * deps (they are not read in the effect body, which is the only thing that rule
 * can see) and left `[isLoading]`, so auto-scroll fired on mount and never
 * again. It shipped that way for months (bug-498). An observer has no
 * dependency array to lose, and it also follows content that grows without any
 * state change in the calling component — the typewriter animation revealing a
 * reply character by character being the case that matters here.
 *
 * @param ref  the scrolling element (the one with `overflow-y-auto`)
 * @returns `onScroll` — attach it to that same element so user scrolling can
 *          unpin and re-pin the view.
 */
export function useStickToBottom(ref: RefObject<HTMLElement | null>) {
  const pinned = useRef(true);

  const onScroll = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < BOTTOM_THRESHOLD_PX;
  }, [ref]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const followContent = () => {
      if (pinned.current) el.scrollTop = el.scrollHeight;
    };

    followContent();
    const observer = new MutationObserver(followContent);
    observer.observe(el, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  }, [ref]);

  return { onScroll };
}
