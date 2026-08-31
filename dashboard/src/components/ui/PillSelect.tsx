import { ChevronDown, type LucideIcon } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

export interface PillOption<T extends string> {
  value: T;
  label: string;
  hint?: string;
}

interface PillSelectProps<T extends string> {
  value: T;
  options: PillOption<T>[];
  onSelect: (value: T) => void;
  icon?: LucideIcon;
  disabled?: boolean;
  /** Marks the pill as accented (e.g. a non-default value is selected). */
  accented?: boolean;
}

const POPOVER_WIDTH = 240; // matches w-60
const POPOVER_MAX_HEIGHT = 320; // matches the old max-h-80
const GAP = 4;
const VIEWPORT_MARGIN = 8;
const EST_ROW_HEIGHT = 48; // label + hint row

interface PopoverPos {
  left: number;
  maxHeight: number;
  /** Anchored from the top (open downward) or the bottom (open upward). */
  top?: number;
  bottom?: number;
}

/**
 * Generic enum selector rendered as a pill button + popover list, matching the
 * `EngineSelector` aesthetic. The popover is portaled to `document.body` and
 * positioned against the viewport (same approach as `EngineSelector`), so it is
 * never clipped by a scroll container or the form footer. Placement auto-flips:
 * it opens downward when there is room, upward otherwise — important because
 * this pill is often rendered near the bottom of the panel (e.g. RecallSection).
 * Deferred-save friendly — it only calls `onSelect`, never touches an API.
 */
export function PillSelect<T extends string>({
  value,
  options,
  onSelect,
  icon: Icon,
  disabled,
  accented,
}: PillSelectProps<T>) {
  const [isOpen, setIsOpen] = useState(false);
  const [pos, setPos] = useState<PopoverPos | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const selected = options.find((o) => o.value === value);
  const pillLabel = selected?.label ?? value;

  // Compute viewport-anchored position, auto-flipping up when there is not
  // enough room below the button.
  const open = useCallback(() => {
    if (!btnRef.current) return;
    const rect = btnRef.current.getBoundingClientRect();
    const estHeight = Math.min(POPOVER_MAX_HEIGHT, options.length * EST_ROW_HEIGHT + VIEWPORT_MARGIN);
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    const openUp = spaceBelow < estHeight + GAP + VIEWPORT_MARGIN && spaceAbove > spaceBelow;
    // Right-align-friendly clamp: keep the popover fully on-screen horizontally.
    const left = Math.max(VIEWPORT_MARGIN, Math.min(rect.left, window.innerWidth - POPOVER_WIDTH - VIEWPORT_MARGIN));
    setPos(
      openUp
        ? {
            left,
            bottom: window.innerHeight - rect.top + GAP,
            maxHeight: Math.min(POPOVER_MAX_HEIGHT, spaceAbove - 12),
          }
        : {
            left,
            top: rect.bottom + GAP,
            maxHeight: Math.min(POPOVER_MAX_HEIGHT, spaceBelow - 12),
          },
    );
    setIsOpen(true);
  }, [options.length]);

  // Close on outside click / Escape.
  useEffect(() => {
    if (!isOpen) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (btnRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setIsOpen(false);
      btnRef.current?.blur();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      // Claim the key so an enclosing Modal does not close as well: dismissing
      // the popover and the dialog around it on one press loses the edit.
      // Capture phase, because both listeners sit on `document` and the modal —
      // mounted first — would otherwise reach its bubble handler first and read
      // defaultPrevented before this ever runs.
      e.preventDefault();
      setIsOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey, true);
    };
  }, [isOpen]);

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        disabled={disabled}
        onClick={() => {
          if (disabled) return;
          if (isOpen) {
            setIsOpen(false);
            btnRef.current?.blur();
          } else {
            open();
          }
        }}
        className={`flex items-center gap-1.5 px-4 py-2.5 rounded-lg text-xs font-mono font-bold uppercase tracking-wider border transition-colors disabled:opacity-30 disabled:cursor-not-allowed ${
          accented && !disabled
            ? 'border-brand/40 bg-brand/10 text-brand'
            : 'border-edge bg-glass text-content-secondary hover:text-brand hover:border-brand/30'
        }`}
        style={{ outline: 'none' }}
      >
        {Icon && <Icon size={14} />}
        <span>{pillLabel}</span>
        {!disabled && <ChevronDown size={12} className={`transition-transform ${isOpen ? 'rotate-180' : ''}`} />}
      </button>

      {isOpen &&
        !disabled &&
        pos &&
        createPortal(
          <div
            ref={menuRef}
            className="fixed w-60 bg-surface-primary/95 backdrop-blur-xl border border-edge rounded-xl shadow-2xl shadow-black/40 overflow-y-auto py-1 z-[9998]"
            style={{ left: pos.left, top: pos.top, bottom: pos.bottom, maxHeight: pos.maxHeight }}
          >
            {options.map((opt) => {
              const isSelected = opt.value === value;
              return (
                <button
                  key={opt.value}
                  type="button"
                  onClick={() => {
                    onSelect(opt.value);
                    setIsOpen(false);
                    btnRef.current?.blur();
                  }}
                  className={`no-focus-ring w-full flex flex-col items-start gap-0.5 px-3 py-2 text-left transition-colors ${
                    isSelected
                      ? 'bg-brand/10 text-brand'
                      : 'text-content-secondary hover:bg-glass hover:text-content-primary'
                  }`}
                >
                  <span className="text-[11px] font-mono font-bold uppercase tracking-wider">{opt.label}</span>
                  {opt.hint && (
                    <span className="text-[10px] text-content-tertiary normal-case font-normal">{opt.hint}</span>
                  )}
                </button>
              );
            })}
          </div>,
          document.body,
        )}
    </>
  );
}
