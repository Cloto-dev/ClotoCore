import { type LucideIcon, X } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

interface ModalProps {
  title: string;
  icon?: LucideIcon;
  size?: 'sm' | 'lg';
  onClose: () => void;
  children: React.ReactNode;
}

// Escape reaches every open modal, so the innermost one has to claim the key:
// the Danger Zone confirm opens on top of the Settings modal, and closing both
// on one press would drop the user two levels deep in a single stroke.
const stack: symbol[] = [];

export function Modal({ title, icon: Icon, size = 'sm', onClose, children }: ModalProps) {
  const isLarge = size === 'lg';
  const { t } = useTranslation();

  // The listener is registered once per mount; reading onClose through a ref
  // keeps a caller's inline arrow from tearing it down and re-adding it on
  // every render, which would silently reorder the stack.
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const id = Symbol('modal');
    stack.push(id);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      // A popover inside the modal (PillSelect) marks the key as spent; taking
      // it here as well would close the dropdown and the modal behind it.
      if (e.defaultPrevented) return;
      if (stack[stack.length - 1] !== id) return;
      e.preventDefault();
      onCloseRef.current();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      const i = stack.indexOf(id);
      if (i !== -1) stack.splice(i, 1);
    };
  }, []);

  // Portal to <body>: `fixed` is resolved against the nearest ancestor with a
  // backdrop-filter/transform (it becomes the containing block), so a modal
  // opened from inside e.g. the backdrop-blurred sidebar would be trapped in
  // and clipped to that ancestor instead of covering the viewport.
  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/[0.35] backdrop-blur-[2px] animate-in fade-in duration-200"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        style={isLarge ? { width: '70vw', height: 'calc(70vw * 9 / 16)' } : undefined}
        className={`flex flex-col rounded-2xl overflow-hidden shadow-2xl border border-edge bg-surface-primary animate-in zoom-in-95 duration-200 ${
          isLarge ? 'max-h-[85vh]' : 'w-[420px] max-h-[80vh]'
        }`}
      >
        {/* Header */}
        <div className="flex items-center gap-2 px-4 py-2.5 border-b border-edge bg-surface-primary shrink-0">
          {Icon && <Icon size={14} className="text-brand" />}
          <span className="text-xs font-mono uppercase tracking-widest text-content-primary">{title}</span>
          <button
            onClick={onClose}
            aria-label={t('close')}
            className="ml-auto p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
          >
            <X size={16} />
          </button>
        </div>

        {/* Content */}
        <div className={`flex-1 min-h-0 ${isLarge ? 'overflow-hidden' : 'overflow-y-auto'}`}>{children}</div>
      </div>
    </div>,
    document.body,
  );
}
