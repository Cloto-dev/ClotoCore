import { useEffect, useRef } from 'react';
import { isTypingTarget, matchesShortcut, SHORTCUTS, type ShortcutId } from '../lib/shortcuts';

/**
 * What a shortcut does, registered by whoever can do it.
 *
 * A handler answers `false` when it cannot act right now (its screen is
 * hidden, say); the next registration is asked then, and a shortcut nobody
 * answers is left to the browser. The most recent registration is asked first.
 */
type Handler = () => unknown;

const registry = new Map<ShortcutId, Handler[]>();
let listening = false;

function onKeyDown(e: KeyboardEvent) {
  // An IME composing text owns the keyboard.
  if (e.defaultPrevented || e.isComposing) return;
  for (const [id, spec] of Object.entries(SHORTCUTS) as [ShortcutId, (typeof SHORTCUTS)[ShortcutId]][]) {
    if (!matchesShortcut(e, spec)) continue;
    // A bare key is text while someone is typing; a modified one is not.
    if (!spec.mod && isTypingTarget(e.target)) return;
    const handlers = registry.get(id) ?? [];
    for (let i = handlers.length - 1; i >= 0; i--) {
      if (handlers[i]() !== false) {
        e.preventDefault();
        return;
      }
    }
    return;
  }
}

export function useShortcut(id: ShortcutId, handler: Handler) {
  const latest = useRef(handler);
  latest.current = handler;

  useEffect(() => {
    const entry: Handler = () => latest.current();
    registry.set(id, [...(registry.get(id) ?? []), entry]);
    if (!listening) {
      window.addEventListener('keydown', onKeyDown);
      listening = true;
    }
    return () => {
      registry.set(
        id,
        (registry.get(id) ?? []).filter((h) => h !== entry),
      );
    };
  }, [id]);
}
