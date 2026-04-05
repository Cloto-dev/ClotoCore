import { Activity, Info, MousePointer, ScrollText, Settings, Shield, Sun, Zap } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InteractiveGrid } from './InteractiveGrid';
import {
  AboutSection,
  AdvancedSection,
  DisplaySection,
  GeneralSection,
  HealthSection,
  LogSection,
  SecuritySection,
} from './settings';
import { ViewHeader } from './ViewHeader';

type Section = 'general' | 'security' | 'display' | 'advanced' | 'health' | 'log' | 'about';

const NAV_ITEMS: { id: Section; labelKey: string; icon: typeof Sun }[] = [
  { id: 'general', labelKey: 'sections.general', icon: Sun },
  { id: 'security', labelKey: 'sections.security', icon: Shield },
  { id: 'display', labelKey: 'sections.display', icon: MousePointer },
  { id: 'advanced', labelKey: 'sections.advanced', icon: Zap },
  { id: 'health', labelKey: 'sections.health', icon: Activity },
  { id: 'log', labelKey: 'sections.log', icon: ScrollText },
  { id: 'about', labelKey: 'sections.about', icon: Info },
];

export function SettingsView({ onBack, initialSection }: { onBack?: () => void; initialSection?: Section }) {
  const [activeSection, setActiveSection] = useState<Section>(initialSection ?? 'general');
  const { t } = useTranslation('settings');

  return (
    <div className="flex flex-col h-full bg-surface-base text-content-primary relative">
      <InteractiveGrid />

      {onBack && (
        <div className="relative z-10">
          <ViewHeader icon={Settings} title={t('title')} onBack={onBack} />
        </div>
      )}

      <div className="relative z-10 flex flex-1 overflow-hidden">
        {/* Sidebar Navigation */}
        <nav className="w-44 border-r border-edge bg-glass-subtle backdrop-blur-sm flex flex-col py-4">
          {NAV_ITEMS.map(({ id, labelKey, icon: Icon }) => (
            <button
              key={id}
              onClick={() => setActiveSection(id)}
              className={`flex items-center gap-3 px-5 py-3 text-sm font-bold tracking-widest uppercase transition-all ${
                activeSection === id
                  ? 'text-brand bg-brand/5 border-r-2 border-brand'
                  : 'text-content-tertiary hover:text-content-secondary hover:bg-surface-secondary'
              }`}
            >
              <Icon size={16} />
              {t(labelKey)}
            </button>
          ))}
        </nav>

        {/* Content Area */}
        <div className="flex-1 overflow-y-auto p-8">
          {activeSection === 'general' && <GeneralSection />}
          {activeSection === 'security' && <SecuritySection />}
          {activeSection === 'display' && <DisplaySection />}
          {activeSection === 'advanced' && <AdvancedSection />}
          {activeSection === 'health' && <HealthSection />}
          {activeSection === 'log' && <LogSection />}
          {activeSection === 'about' && <AboutSection />}
        </div>
      </div>
    </div>
  );
}
