import { fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { isTypingTarget, matchesShortcut, SHORTCUTS } from '../../lib/shortcuts';
import { useShortcut } from '../useShortcut';

function Registers({ id, handler }: { id: 'palette' | 'newChat' | 'focusComposer'; handler: () => unknown }) {
  useShortcut(id, handler);
  return <input aria-label="field" />;
}

describe('the shortcut table', () => {
  it('wants the modifier exactly when the shortcut has one', () => {
    expect(matchesShortcut({ key: 'k', metaKey: true, ctrlKey: false, altKey: false }, SHORTCUTS.palette)).toBe(true);
    expect(matchesShortcut({ key: 'K', metaKey: false, ctrlKey: true, altKey: false }, SHORTCUTS.palette)).toBe(true);
    expect(matchesShortcut({ key: 'k', metaKey: false, ctrlKey: false, altKey: false }, SHORTCUTS.palette)).toBe(false);
    expect(matchesShortcut({ key: 'k', metaKey: true, ctrlKey: false, altKey: true }, SHORTCUTS.palette)).toBe(false);
    expect(matchesShortcut({ key: '/', metaKey: false, ctrlKey: false, altKey: false }, SHORTCUTS.focusComposer)).toBe(
      true,
    );
    expect(matchesShortcut({ key: '/', metaKey: true, ctrlKey: false, altKey: false }, SHORTCUTS.focusComposer)).toBe(
      false,
    );
  });

  it('counts a text field, a text area and editable text as typing, and a button or checkbox as not', () => {
    const text = document.createElement('input');
    const check = document.createElement('input');
    check.type = 'checkbox';
    const area = document.createElement('textarea');
    const button = document.createElement('button');
    const editable = document.createElement('div');
    editable.contentEditable = 'true';
    // jsdom does not derive isContentEditable from the attribute.
    Object.defineProperty(editable, 'isContentEditable', { value: true });
    expect([text, area, editable].map(isTypingTarget)).toEqual([true, true, true]);
    expect([check, button, null].map(isTypingTarget)).toEqual([false, false, false]);
  });
});

describe('useShortcut', () => {
  it('asks the newest registration first, and the one before when it cannot act', () => {
    const older = vi.fn(() => true);
    const newer = vi.fn(() => false);
    render(
      <>
        <Registers id="newChat" handler={older} />
        <Registers id="newChat" handler={newer} />
      </>,
    );
    const event = new KeyboardEvent('keydown', { key: 'n', metaKey: true, cancelable: true });
    window.dispatchEvent(event);
    expect(newer).toHaveBeenCalledTimes(1);
    expect(older).toHaveBeenCalledTimes(1);
    expect(newer.mock.invocationCallOrder[0]).toBeLessThan(older.mock.invocationCallOrder[0]);
    expect(event.defaultPrevented).toBe(true);
  });

  it('leaves the key to the browser when nobody could act on it', () => {
    render(<Registers id="newChat" handler={() => false} />);
    const event = new KeyboardEvent('keydown', { key: 'n', metaKey: true, cancelable: true });
    window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it('does not take a bare key from someone typing, but does take a modified one', () => {
    const focus = vi.fn();
    const palette = vi.fn();
    const { getAllByLabelText } = render(
      <>
        <Registers id="focusComposer" handler={focus} />
        <Registers id="palette" handler={palette} />
      </>,
    );
    const field = getAllByLabelText('field')[0];
    fireEvent.keyDown(field, { key: '/' });
    expect(focus).not.toHaveBeenCalled();
    fireEvent.keyDown(field, { key: 'k', ctrlKey: true });
    expect(palette).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(document.body, { key: '/' });
    expect(focus).toHaveBeenCalledTimes(1);
  });

  it('ignores keys while an input method is composing', () => {
    const palette = vi.fn();
    render(<Registers id="palette" handler={palette} />);
    fireEvent.keyDown(window, { key: 'k', metaKey: true, isComposing: true });
    expect(palette).not.toHaveBeenCalled();
  });

  it('forgets a registration when its component goes away', () => {
    const handler = vi.fn();
    const { unmount } = render(<Registers id="palette" handler={handler} />);
    unmount();
    fireEvent.keyDown(window, { key: 'k', metaKey: true });
    expect(handler).not.toHaveBeenCalled();
  });

  it('calls the latest handler a component passed, not the one from its first render', () => {
    const first = vi.fn();
    const second = vi.fn();
    const { rerender } = render(<Registers id="palette" handler={first} />);
    rerender(<Registers id="palette" handler={second} />);
    fireEvent.keyDown(window, { key: 'k', metaKey: true });
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });
});
