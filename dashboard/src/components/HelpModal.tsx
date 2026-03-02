import { X, Activity, Database, MessageSquare, Puzzle, Clock, Settings, MessageCircle, ExternalLink } from 'lucide-react';

interface HelpModalProps {
  onClose: () => void;
  onAskAgent: () => void;
}

const menuGuide = [
  { icon: Activity, label: 'STATUS', desc: 'System health, agent status, and live metrics' },
  { icon: Database, label: 'MEMORY', desc: 'Browse stored memories and episode archives' },
  { icon: MessageSquare, label: 'CLOTO', desc: 'Chat with your AI agents' },
  { icon: Puzzle, label: 'MCP', desc: 'Manage MCP server connections and tools' },
  { icon: Clock, label: 'CRON', desc: 'Schedule automated agent tasks' },
  { icon: Settings, label: 'SETTINGS', desc: 'Configure API keys, themes, and preferences' },
];

export function HelpModal({ onClose, onAskAgent }: HelpModalProps) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-[var(--surface-overlay)] backdrop-blur-sm"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div className="bg-surface-primary rounded-2xl shadow-2xl w-[420px] max-h-[80vh] overflow-y-auto animate-in fade-in zoom-in-95 duration-200">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-edge">
          <h2 className="text-sm font-mono font-bold tracking-widest text-content-primary uppercase">Cloto Help</h2>
          <button onClick={onClose} className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors">
            <X size={16} />
          </button>
        </div>

        {/* Menu Guide */}
        <div className="px-5 py-4 space-y-3">
          <p className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary mb-3">Navigation</p>
          {menuGuide.map(({ icon: Icon, label, desc }) => (
            <div key={label} className="flex items-start gap-3">
              <Icon size={14} className="text-brand mt-0.5 shrink-0" />
              <div>
                <span className="text-xs font-mono font-bold text-content-primary">{label}</span>
                <span className="text-xs text-content-secondary ml-2">{desc}</span>
              </div>
            </div>
          ))}
        </div>

        {/* Divider */}
        <div className="border-t border-edge mx-5" />

        {/* Ask Agent */}
        <div className="px-5 py-4">
          <button
            onClick={onAskAgent}
            className="w-full flex items-center justify-center gap-2 px-4 py-2.5 rounded-lg bg-brand/10 hover:bg-brand/20 border border-brand/30 text-brand text-xs font-mono font-bold tracking-wide transition-colors"
          >
            <MessageCircle size={14} />
            Ask Cloto Assistant
          </button>
          <p className="text-[9px] text-content-tertiary text-center mt-2 font-mono">
            Chat with the default agent for help and questions
          </p>
        </div>

        {/* Footer */}
        <div className="border-t border-edge px-5 py-3 flex items-center justify-between">
          <div className="flex items-center gap-3 text-[9px] font-mono text-content-tertiary">
            <a href="https://github.com/Cloto-dev/ClotoCore" target="_blank" rel="noopener noreferrer" className="hover:text-content-secondary transition-colors flex items-center gap-1">
              Docs <ExternalLink size={8} />
            </a>
            <a href="https://github.com/Cloto-dev/ClotoCore/issues" target="_blank" rel="noopener noreferrer" className="hover:text-content-secondary transition-colors flex items-center gap-1">
              GitHub <ExternalLink size={8} />
            </a>
          </div>
          <span className="text-[9px] font-mono text-content-tertiary">v{__APP_VERSION__}</span>
        </div>
      </div>
    </div>
  );
}

declare const __APP_VERSION__: string;
