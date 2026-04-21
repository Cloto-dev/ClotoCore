import { Eye, EyeOff } from 'lucide-react';
import { useState } from 'react';

interface SecretInputProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  className?: string;
  onKeyDown?: (e: React.KeyboardEvent) => void;
}

export function SecretInput({ value, onChange, placeholder, className, onKeyDown }: SecretInputProps) {
  const [visible, setVisible] = useState(false);

  return (
    <div className="relative flex-1">
      <input
        type={visible ? 'text' : 'password'}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        onKeyDown={onKeyDown}
        className={
          className ??
          'w-full text-xs font-mono bg-surface-secondary border border-edge rounded px-2 py-1 pr-7 text-content-primary placeholder:text-content-tertiary focus:outline-none focus:border-brand transition-colors'
        }
      />
      <button
        type="button"
        onClick={() => setVisible((v) => !v)}
        aria-label={visible ? 'Hide value' : 'Show value'}
        className="absolute right-1.5 top-1/2 -translate-y-1/2 text-content-tertiary hover:text-content-secondary"
      >
        {visible ? <EyeOff size={12} /> : <Eye size={12} />}
      </button>
    </div>
  );
}
