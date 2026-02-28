import { Sun, Moon, Monitor } from 'lucide-react';
import { SectionCard } from './common';
import { useTheme } from '../../hooks/useTheme';

export function GeneralSection() {
  const { preference, setPreference } = useTheme();
  const themes: { value: 'light' | 'dark' | 'system'; icon: typeof Sun; label: string }[] = [
    { value: 'light', icon: Sun, label: 'Light' },
    { value: 'dark', icon: Moon, label: 'Dark' },
    { value: 'system', icon: Monitor, label: 'System' },
  ];

  return (
    <>
      <SectionCard title="Theme">
        <div className="flex gap-3">
          {themes.map(({ value, icon: Icon, label }) => (
            <button
              key={value}
              onClick={() => setPreference(value)}
              className={`flex items-center gap-2 px-5 py-2.5 rounded-xl text-xs font-bold transition-all ${
                preference === value
                  ? 'bg-brand text-white shadow-md'
                  : 'bg-surface-secondary text-content-secondary hover:text-content-primary border border-edge hover:border-brand'
              }`}
            >
              <Icon size={14} />
              {label}
            </button>
          ))}
        </div>
      </SectionCard>

      <SectionCard title="Version">
        <div className="flex items-center gap-3">
          <span className="text-2xl font-mono font-black text-brand">v{__APP_VERSION__}</span>
          <span className="text-[10px] text-content-tertiary font-mono uppercase tracking-widest">Beta 2</span>
        </div>
      </SectionCard>
    </>
  );
}
