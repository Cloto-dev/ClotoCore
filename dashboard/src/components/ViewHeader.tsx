import { Link } from 'react-router-dom';
import { ArrowLeft, Minus, Square, X, type LucideIcon } from 'lucide-react';
import { isTauri, minimizeWindow, toggleMaximizeWindow, closeWindow } from '../lib/tauri';

interface ViewHeaderProps {
  icon: LucideIcon;
  title: string;
  onBack?: (() => void) | string;
  right?: React.ReactNode;
}

export function ViewHeader({ icon: Icon, title, onBack, right }: ViewHeaderProps) {
  return (
    <header
      className="flex items-center gap-3 px-4 py-2 border-b border-edge bg-surface-primary select-none"
      data-tauri-drag-region=""
    >
      {typeof onBack === 'string' ? (
        <Link to={onBack} className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors">
          <ArrowLeft size={16} />
        </Link>
      ) : onBack ? (
        <button onClick={onBack} className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors">
          <ArrowLeft size={16} />
        </button>
      ) : null}
      <Icon size={14} className="text-brand" />
      <h1 className="text-xs font-mono uppercase tracking-widest text-content-primary">{title}</h1>
      {right && <div className="ml-auto flex items-center gap-3">{right}</div>}

      {/* Window Controls (Tauri only) */}
      {isTauri && (
        <div className={`flex items-center gap-2 pr-1 ${right ? '' : 'ml-auto'}`}>
          <button onClick={minimizeWindow} className="p-1.5 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors">
            <Minus size={14} />
          </button>
          <button onClick={toggleMaximizeWindow} className="p-1.5 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors">
            <Square size={12} />
          </button>
          <button onClick={closeWindow} className="p-1.5 ml-1 rounded hover:bg-red-500/20 text-content-tertiary hover:text-red-500 transition-colors">
            <X size={14} />
          </button>
        </div>
      )}
    </header>
  );
}
