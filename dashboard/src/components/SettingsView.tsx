import { useState } from 'react';
import { Sun, Shield, MousePointer, ScrollText, Info, Zap, Settings } from 'lucide-react';
import { ViewHeader } from './ViewHeader';
import { GeneralSection, SecuritySection, DisplaySection, AdvancedSection, LogSection, AboutSection } from './settings';

type Section = 'general' | 'security' | 'display' | 'advanced' | 'log' | 'about';

const NAV_ITEMS: { id: Section; label: string; icon: typeof Sun }[] = [
  { id: 'general', label: 'GENERAL', icon: Sun },
  { id: 'security', label: 'SECURITY', icon: Shield },
  { id: 'display', label: 'DISPLAY', icon: MousePointer },
  { id: 'advanced', label: 'ADVANCED', icon: Zap },
  { id: 'log', label: 'LOG', icon: ScrollText },
  { id: 'about', label: 'ABOUT', icon: Info },
];

export function SettingsView({ onBack }: { onBack?: () => void }) {
  const [activeSection, setActiveSection] = useState<Section>('general');

  return (
    <div className="flex flex-col h-full bg-surface-base text-content-primary relative">
      {/* Background grid — MemoryCore aesthetic */}
      <div
        className="absolute inset-0 z-0 opacity-30 pointer-events-none"
        style={{
          backgroundImage: `linear-gradient(to right, var(--canvas-grid) 1px, transparent 1px), linear-gradient(to bottom, var(--canvas-grid) 1px, transparent 1px)`,
          backgroundSize: '40px 40px',
          maskImage: 'linear-gradient(to bottom, black 40%, transparent 100%)',
          WebkitMaskImage: 'linear-gradient(to bottom, black 40%, transparent 100%)',
        }}
      />

      {onBack && (
        <div className="relative z-10">
          <ViewHeader icon={Settings} title="Settings" onBack={onBack} />
        </div>
      )}

      <div className="relative z-10 flex flex-1 overflow-hidden">
      {/* Sidebar Navigation */}
      <nav className="w-44 border-r border-edge bg-glass-subtle backdrop-blur-sm flex flex-col py-4">
        {NAV_ITEMS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setActiveSection(id)}
            className={`flex items-center gap-3 px-5 py-3 text-[10px] font-bold tracking-widest uppercase transition-all ${
              activeSection === id
                ? 'text-brand bg-brand/5 border-r-2 border-brand'
                : 'text-content-tertiary hover:text-content-secondary hover:bg-surface-secondary'
            }`}
          >
            <Icon size={14} />
            {label}
          </button>
        ))}
      </nav>

      {/* Content Area */}
      <div className="flex-1 overflow-y-auto p-8">
        {activeSection === 'general' && <GeneralSection />}
        {activeSection === 'security' && <SecuritySection />}
        {activeSection === 'display' && <DisplaySection />}
        {activeSection === 'advanced' && <AdvancedSection />}
        {activeSection === 'log' && <LogSection />}
        {activeSection === 'about' && <AboutSection />}
      </div>
      </div>
    </div>
  );
}
