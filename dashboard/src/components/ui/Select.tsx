import { useCallback, useEffect, useId, useRef, useState } from 'react';

/**
 * The workshop's picker (docs/gui/samples/08-agent-settings.html, `.sel`).
 *
 * A native `<select>` cannot be drawn: the closed control inherits the host's
 * widget metrics and the open list is painted by the platform, so neither the
 * density nor the surface steps this product decides apply to it. This is the
 * same control as a button plus a listbox, with the keyboard contract the
 * native one has (arrows move the active option, Enter takes it, Escape
 * abandons, a click elsewhere closes).
 */

export interface SelectOption<T extends string> {
  value: T;
  label: string;
  /** Second line under the label, when the value needs one. */
  hint?: string;
}

interface Props<T extends string> {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  /** The accessible name — this control has no visible label of its own. */
  label: string;
  /** Shown when `value` matches no option (nothing chosen yet). */
  placeholder?: string;
  disabled?: boolean;
  className?: string;
}

export function Select<T extends string>({
  value,
  options,
  onChange,
  label,
  placeholder = '',
  disabled = false,
  className = '',
}: Props<T>) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const listId = useId();

  const selectedIndex = options.findIndex((o) => o.value === value);
  const selected = selectedIndex >= 0 ? options[selectedIndex] : null;

  const openList = useCallback(() => {
    if (disabled) return;
    setActive(selectedIndex >= 0 ? selectedIndex : 0);
    setOpen(true);
  }, [disabled, selectedIndex]);

  // Close on a click anywhere else. Pointerdown rather than click so the list
  // is gone before whatever was clicked reacts to it.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (rootRef.current?.contains(e.target as Node)) return;
      setOpen(false);
    };
    document.addEventListener('pointerdown', onPointerDown);
    return () => document.removeEventListener('pointerdown', onPointerDown);
  }, [open]);

  const choose = (index: number) => {
    const option = options[index];
    if (!option) return;
    onChange(option.value);
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (disabled) return;
    if (!open) {
      if (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        openList();
      }
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((i) => (i + 1) % options.length);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((i) => (i - 1 + options.length) % options.length);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      choose(active);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      // Stopped here so a Select inside a dialog does not also close it: the
      // innermost open thing is the one Escape is about.
      e.stopPropagation();
      setOpen(false);
    }
  };

  return (
    <div className={`sel-wrap ${className}`.trim()} ref={rootRef}>
      <button
        type="button"
        className="sel"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        disabled={disabled}
        onClick={() => (open ? setOpen(false) : openList())}
        onKeyDown={onKeyDown}
      >
        <span className="lbl">{selected ? selected.label : placeholder}</span>
        <span className="c" aria-hidden="true">
          ⌄
        </span>
      </button>
      {open && (
        <div className="sel-list" id={listId} role="listbox" aria-label={label}>
          {options.map((option, i) => (
            <div
              key={option.value}
              id={`${listId}-${i}`}
              role="option"
              tabIndex={-1}
              aria-selected={option.value === value}
              className={`sel-opt${i === active ? ' active' : ''}${option.value === value ? ' on' : ''}`}
              onPointerDown={(e) => {
                e.preventDefault();
                choose(i);
              }}
              onPointerEnter={() => setActive(i)}
            >
              <span className="lbl">{option.label}</span>
              {option.hint && <small>{option.hint}</small>}
            </div>
          ))}
          {options.length === 0 && <div className="sel-empty">{placeholder}</div>}
        </div>
      )}
    </div>
  );
}
