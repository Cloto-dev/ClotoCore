export function Toggle({ enabled, onToggle, label }: { enabled: boolean; onToggle: () => void; label: string }) {
  return (
    <label className="flex items-center justify-between cursor-pointer select-none group">
      <span className="text-xs text-content-secondary group-hover:text-content-primary transition-colors">{label}</span>
      <button
        onClick={onToggle}
        className={`w-10 h-5 rounded-full transition-colors relative ${enabled ? 'bg-brand' : 'bg-surface-secondary border border-edge'}`}
      >
        <div className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-5' : 'translate-x-0.5'}`} />
      </button>
    </label>
  );
}

export function SectionCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="bg-glass-strong backdrop-blur-sm p-5 rounded-lg border border-edge shadow-sm mb-4">
      <h3 className="text-[10px] font-black text-content-tertiary uppercase tracking-[0.2em] mb-4">{title}</h3>
      {children}
    </div>
  );
}
