/**
 * Every keyboard shortcut the dashboard answers anywhere, in one table.
 *
 * A screen registers what a shortcut does with `useShortcut`; the keys live
 * here so one place says which combinations are taken, and one listener
 * decides which registration answers.
 */

export type ShortcutId = 'palette' | 'newChat' | 'focusComposer';

export interface ShortcutSpec {
  /** `KeyboardEvent.key`, lower case. */
  key: string;
  /** ⌘ on macOS, Ctrl elsewhere. */
  mod: boolean;
}

export const SHORTCUTS: Record<ShortcutId, ShortcutSpec> = {
  palette: { key: 'k', mod: true },
  newChat: { key: 'n', mod: true },
  focusComposer: { key: '/', mod: false },
};

export function matchesShortcut(
  e: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'altKey'>,
  spec: ShortcutSpec,
): boolean {
  if (e.altKey) return false;
  if ((e.metaKey || e.ctrlKey) !== spec.mod) return false;
  return e.key.toLowerCase() === spec.key;
}

/** Whether a key pressed here is text being written, which a bare shortcut must leave alone. */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return !['button', 'checkbox', 'radio', 'range', 'submit', 'reset', 'file', 'color'].includes(target.type);
  }
  return false;
}
