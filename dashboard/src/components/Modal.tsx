import { X, type LucideIcon } from 'lucide-react';

interface ModalProps {
  title: string;
  icon?: LucideIcon;
  size?: 'sm' | 'lg';
  onClose: () => void;
  children: React.ReactNode;
}

export function Modal({ title, icon: Icon, size = 'sm', onClose, children }: ModalProps) {
  const isLarge = size === 'lg';

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/[0.35] backdrop-blur-[2px] animate-in fade-in duration-200"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div
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
            className="ml-auto p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
          >
            <X size={16} />
          </button>
        </div>

        {/* Content */}
        <div className={`flex-1 min-h-0 ${isLarge ? 'overflow-hidden' : 'overflow-y-auto'}`}>
          {children}
        </div>
      </div>
    </div>
  );
}
