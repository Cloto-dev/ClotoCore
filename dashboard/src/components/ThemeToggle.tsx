import { Sun, Moon, Monitor } from 'lucide-react';
import { useTheme } from '../hooks/useTheme';

export function ThemeToggle() {
  const { preference, setPreference } = useTheme();

  const options = [
    { value: 'light' as const, icon: Sun, label: 'Light' },
    { value: 'dark' as const, icon: Moon, label: 'Dark' },
    { value: 'system' as const, icon: Monitor, label: 'System' },
  ];

  return (
    <div className="flex items-center gap-1 p-1 rounded-xl bg-surface-secondary border border-edge">
      {options.map(({ value, icon: Icon, label }) => (
        <button
          key={value}
          onClick={() => setPreference(value)}
          className={`p-1.5 rounded-lg transition-all ${
            preference === value
              ? 'bg-brand text-white shadow-sm'
              : 'text-content-tertiary hover:text-content-secondary'
          }`}
          title={label}
        >
          <Icon size={14} />
        </button>
      ))}
    </div>
  );
}
