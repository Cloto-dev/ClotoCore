import { Eye, EyeOff, Plus, X } from 'lucide-react';
import { useState } from 'react';

export interface EnvEntry {
  key: string;
  value: string;
}

interface EnvVariableEditorProps {
  entries: EnvEntry[];
  onChange: (entries: EnvEntry[]) => void;
  placeholderKey?: string;
  placeholderValue?: string;
  removeLabel?: string;
  addLabel?: string;
  emptyHint?: string;
}

export function EnvVariableEditor({
  entries,
  onChange,
  placeholderKey = 'KEY',
  placeholderValue = 'value',
  removeLabel = 'Remove',
  addLabel = 'Add',
  emptyHint,
}: EnvVariableEditorProps) {
  const [visibleKeys, setVisibleKeys] = useState<Set<string>>(new Set());
  const [newKey, setNewKey] = useState('');
  const [newValue, setNewValue] = useState('');

  const toggleVisibility = (key: string) => {
    setVisibleKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const updateValue = (key: string, value: string) => {
    onChange(entries.map((e) => (e.key === key ? { ...e, value } : e)));
  };

  const removeEntry = (key: string) => {
    onChange(entries.filter((e) => e.key !== key));
  };

  const addEntry = () => {
    const trimmedKey = newKey.trim();
    if (!trimmedKey) return;
    if (entries.some((e) => e.key === trimmedKey)) return;
    onChange([...entries, { key: trimmedKey, value: newValue }]);
    setNewKey('');
    setNewValue('');
  };

  return (
    <div className="space-y-2">
      {entries.map((entry) => (
        <div key={entry.key} className="flex items-center gap-2">
          <span className="text-[10px] font-mono text-content-secondary w-40 truncate shrink-0" title={entry.key}>
            {entry.key}
          </span>
          <div className="relative flex-1">
            <input
              type={visibleKeys.has(entry.key) ? 'text' : 'password'}
              value={entry.value === '***' ? '' : entry.value}
              onChange={(e) => updateValue(entry.key, e.target.value || '***')}
              placeholder={entry.value === '***' ? '••••••• (saved)' : ''}
              className="w-full text-xs font-mono bg-surface-secondary border border-edge rounded px-2 py-1 pr-7 text-content-primary placeholder:text-content-tertiary focus:outline-none focus:border-brand transition-colors"
            />
            <button
              type="button"
              onClick={() => toggleVisibility(entry.key)}
              aria-label={visibleKeys.has(entry.key) ? 'Hide value' : 'Show value'}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 text-content-tertiary hover:text-content-secondary"
            >
              {visibleKeys.has(entry.key) ? <EyeOff size={12} /> : <Eye size={12} />}
            </button>
          </div>
          <button
            onClick={() => removeEntry(entry.key)}
            className="p-1 rounded text-content-tertiary hover:text-red-500 hover:bg-red-500/10 transition-colors shrink-0"
            title={removeLabel}
            aria-label={removeLabel}
          >
            <X size={12} />
          </button>
        </div>
      ))}

      {/* Add new variable */}
      <div className="flex items-center gap-2 pt-1 border-t border-edge/50">
        <input
          type="text"
          value={newKey}
          onChange={(e) => setNewKey(e.target.value.toUpperCase())}
          placeholder={placeholderKey}
          className="w-40 text-[10px] font-mono bg-surface-secondary border border-edge rounded px-2 py-1 text-content-primary placeholder:text-content-tertiary focus:outline-none focus:border-brand transition-colors shrink-0"
          onKeyDown={(e) => e.key === 'Enter' && addEntry()}
        />
        <input
          type="password"
          value={newValue}
          onChange={(e) => setNewValue(e.target.value)}
          placeholder={placeholderValue}
          className="flex-1 text-xs font-mono bg-surface-secondary border border-edge rounded px-2 py-1 text-content-primary placeholder:text-content-tertiary focus:outline-none focus:border-brand transition-colors"
          onKeyDown={(e) => e.key === 'Enter' && addEntry()}
        />
        <button
          onClick={addEntry}
          disabled={!newKey.trim()}
          className="p-1 rounded text-brand hover:bg-brand/10 transition-colors disabled:opacity-30 disabled:cursor-not-allowed shrink-0"
          title={addLabel}
          aria-label={addLabel}
        >
          <Plus size={14} />
        </button>
      </div>

      {entries.length === 0 && emptyHint && (
        <p className="text-[9px] font-mono text-content-tertiary py-2">{emptyHint}</p>
      )}
    </div>
  );
}
