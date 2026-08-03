import { renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { useStickToBottom } from '../useStickToBottom';

/**
 * jsdom has no layout, so every geometry property reads 0 and a container is
 * trivially "at the bottom" — a fixture that leaves it that way can never
 * observe the unpinned branch. These helpers give the element a viewport and a
 * content height that grows as turns are appended, with turns deliberately
 * taller than the viewport so "the user scrolled up" is a reachable state.
 */
const VIEWPORT_PX = 300;
const TURN_PX = 400;

function makeContainer() {
  const el = document.createElement('div');
  let contentHeight = 0;
  Object.defineProperty(el, 'clientHeight', { get: () => VIEWPORT_PX, configurable: true });
  Object.defineProperty(el, 'scrollHeight', { get: () => contentHeight, configurable: true });
  document.body.appendChild(el);
  return {
    el,
    ref: { current: el as HTMLElement | null },
    /** Append a turn to the stream, growing the scrollable content. */
    appendTurn(text: string) {
      contentHeight += TURN_PX;
      const turn = document.createElement('p');
      turn.appendChild(document.createTextNode(text));
      el.appendChild(turn);
      return turn;
    },
    /**
     * Extend an existing turn's text in place — how React reveals a streaming
     * reply into a text-only node, and a characterData mutation rather than a
     * childList one.
     */
    growTurn(turn: HTMLElement, text: string, px: number) {
      contentHeight += px;
      (turn.firstChild as Text).data = text;
    },
  };
}

/** MutationObserver callbacks are delivered asynchronously. */
const flushMutations = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('useStickToBottom', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  it('scrolls to the newest turn when one is appended', async () => {
    const { el, ref, appendTurn } = makeContainer();
    renderHook(() => useStickToBottom(ref));

    appendTurn('hello');
    await flushMutations();

    expect(el.scrollTop).toBe(el.scrollHeight);
  });

  it('follows text that grows in place (typewriter reveal)', async () => {
    const { el, ref, appendTurn, growTurn } = makeContainer();
    renderHook(() => useStickToBottom(ref));

    const turn = appendTurn('a');
    await flushMutations();
    expect(el.scrollTop).toBe(TURN_PX);

    growTurn(turn, 'a much longer reply', 500);
    await flushMutations();

    // A dependency-array formulation cannot see this: no state changed in the
    // calling component and no child was added or removed.
    expect(el.scrollTop).toBe(TURN_PX + 500);
  });

  it('leaves the view alone once the user scrolls up', async () => {
    const { el, ref, appendTurn } = makeContainer();
    const { result } = renderHook(() => useStickToBottom(ref));

    appendTurn('first');
    await flushMutations();

    // User scrolls well above the bottom (one full turn of slack).
    el.scrollTop = 0;
    result.current.onScroll();

    appendTurn('second');
    await flushMutations();

    expect(el.scrollTop).toBe(0);
  });

  it('re-pins when the user scrolls back down', async () => {
    const { el, ref, appendTurn } = makeContainer();
    const { result } = renderHook(() => useStickToBottom(ref));

    appendTurn('first');
    await flushMutations();

    el.scrollTop = 0;
    result.current.onScroll();

    // Back to the bottom, then a new turn arrives.
    el.scrollTop = el.scrollHeight - VIEWPORT_PX;
    result.current.onScroll();
    appendTurn('second');
    await flushMutations();

    expect(el.scrollTop).toBe(el.scrollHeight);
  });

  it('stops observing after unmount', async () => {
    const { el, ref, appendTurn } = makeContainer();
    const { unmount } = renderHook(() => useStickToBottom(ref));

    appendTurn('first');
    await flushMutations();

    unmount();
    el.scrollTop = 0;
    appendTurn('after unmount');
    await flushMutations();

    expect(el.scrollTop).toBe(0);
  });
});
