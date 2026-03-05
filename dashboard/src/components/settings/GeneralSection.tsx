import { Sun, Moon, Monitor } from 'lucide-react';
import { SectionCard } from './common';
import { useTheme } from '../../hooks/useTheme';
import { useUserIdentity } from '../../contexts/UserIdentityContext';

export function GeneralSection() {
  const { preference, setPreference } = useTheme();
  const { identity, setIdentity } = useUserIdentity();
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

      <SectionCard title="User Identity">
        <div className="space-y-3">
          <div>
            <label className="text-[10px] text-content-tertiary font-bold uppercase tracking-widest block mb-1">Display Name</label>
            <input
              type="text"
              value={identity.name}
              onChange={e => setIdentity(identity.id, e.target.value)}
              className="w-full px-3 py-2 bg-surface-secondary border border-edge rounded-lg text-sm text-content-primary focus:border-brand focus:outline-none transition-colors"
              placeholder="User"
            />
          </div>
          <p className="text-[10px] text-content-muted">
            The name shown to agents when you chat. Agents can use it to address you personally.
          </p>
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
