import { ChevronDown, type LucideIcon } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

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

/**
 * Generic enum selector rendered as a pill button + popover list, matching the
 * `EngineSelector` aesthetic. Self-contained (no portal): the popover opens
 * below the button and scrolls with the surrounding form. Deferred-save
 * friendly — it only calls `onSelect`, never touches an API.
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
  const wrapRef = useRef<HTMLDivElement>(null);

  const selected = options.find((o) => o.value === value);
  const pillLabel = selected?.label ?? value;

  // Close on outside click / Escape.
  useEffect(() => {
    if (!isOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setIsOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setIsOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [isOpen]);

  return (
    <div ref={wrapRef} className="relative">
      <button
        type="button"
        disabled={disabled}
        onClick={() => setIsOpen((v) => !v)}
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

      {isOpen && !disabled && (
        <div className="absolute left-0 top-full mt-1 w-60 bg-surface-primary/95 backdrop-blur-xl border border-edge rounded-xl shadow-2xl shadow-black/40 overflow-y-auto py-1 z-50 max-h-80">
          {options.map((opt) => {
            const isSelected = opt.value === value;
            return (
              <button
                key={opt.value}
                type="button"
                onClick={() => {
                  onSelect(opt.value);
                  setIsOpen(false);
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
        </div>
      )}
    </div>
  );
}
