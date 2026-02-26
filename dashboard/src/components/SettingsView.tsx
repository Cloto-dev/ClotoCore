import { useState, useEffect, useRef } from 'react';
import { Sun, Moon, Monitor, Eye, EyeOff, MousePointer, ScrollText, Info, Shield, AlertTriangle } from 'lucide-react';
import { useTheme } from '../hooks/useTheme';
import { useApiKey } from '../contexts/ApiKeyContext';
import { useEventStream } from '../hooks/useEventStream';
import { EVENTS_URL, api } from '../services/api';

type Section = 'general' | 'security' | 'display' | 'log' | 'about';

const NAV_ITEMS: { id: Section; label: string; icon: typeof Sun }[] = [
  { id: 'general', label: 'GENERAL', icon: Sun },
  { id: 'security', label: 'SECURITY', icon: Shield },
  { id: 'display', label: 'DISPLAY', icon: MousePointer },
  { id: 'log', label: 'LOG', icon: ScrollText },
  { id: 'about', label: 'ABOUT', icon: Info },
];

function Toggle({ enabled, onToggle, label }: { enabled: boolean; onToggle: () => void; label: string }) {
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

export function SettingsView() {
  const [activeSection, setActiveSection] = useState<Section>('general');

  return (
    <div className="flex h-full bg-surface-base text-content-primary">
      {/* Sidebar Navigation */}
      <nav className="w-44 border-r border-edge bg-surface-primary flex flex-col py-4">
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
        {activeSection === 'log' && <LogSection />}
        {activeSection === 'about' && <AboutSection />}
      </div>
    </div>
  );
}

function SectionCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="bg-glass-strong backdrop-blur-sm p-5 rounded-2xl border border-edge shadow-sm mb-4">
      <h3 className="text-[10px] font-black text-content-tertiary uppercase tracking-[0.2em] mb-4">{title}</h3>
      {children}
    </div>
  );
}

/* ======================== GENERAL ======================== */

function GeneralSection() {
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

/* ======================== SECURITY ======================== */

function SecuritySection() {
  const { apiKey, isPersisted, setApiKey, setPersist, forgetApiKey } = useApiKey();
  const [newKey, setNewKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [confirmInvalidate, setConfirmInvalidate] = useState(false);

  const handleSave = async () => {
    if (!newKey.trim()) return;
    setSaving(true);
    setError('');
    try {
      await api.applyPluginSettings([], newKey.trim());
      setApiKey(newKey.trim());
      setNewKey('');
    } catch {
      setError('Invalid API key');
    } finally {
      setSaving(false);
    }
  };

  const handleInvalidate = async () => {
    if (!apiKey) return;
    try {
      await api.invalidateApiKey(apiKey);
      forgetApiKey();
      setConfirmInvalidate(false);
    } catch {
      setError('Failed to invalidate key');
    }
  };

  return (
    <>
      <SectionCard title="API Key">
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <div className={`w-2 h-2 rounded-full ${apiKey ? 'bg-green-500' : 'bg-amber-500'}`} />
            <span className="text-xs text-content-secondary">{apiKey ? 'Configured' : 'Not configured'}</span>
          </div>

          <div className="flex gap-2">
            <div className="relative flex-1">
              <input
                type={showKey ? 'text' : 'password'}
                value={newKey}
                onChange={e => { setNewKey(e.target.value); setError(''); }}
                placeholder={apiKey ? 'Enter new key to replace' : 'Enter API key'}
                className="w-full bg-surface-secondary border border-edge rounded-lg px-3 py-2 text-xs font-mono text-content-primary placeholder:text-content-muted focus:outline-none focus:border-brand transition-colors"
              />
              <button
                onClick={() => setShowKey(v => !v)}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-content-muted hover:text-content-secondary"
              >
                {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
            <button
              onClick={handleSave}
              disabled={!newKey.trim() || saving}
              className="px-4 py-2 bg-brand text-white text-xs font-bold rounded-lg disabled:opacity-40 hover:bg-brand/90 transition-colors"
            >
              {saving ? '...' : 'Save'}
            </button>
          </div>

          {error && (
            <div className="flex items-center gap-2 text-red-400 text-[10px]">
              <AlertTriangle size={12} />
              {error}
            </div>
          )}

          <Toggle
            enabled={isPersisted}
            onToggle={() => setPersist(!isPersisted)}
            label="Save to this device"
          />

          {apiKey && (
            <div className="pt-3 border-t border-edge">
              {!confirmInvalidate ? (
                <button
                  onClick={() => setConfirmInvalidate(true)}
                  className="text-[10px] text-red-400 hover:text-red-300 font-bold uppercase tracking-widest transition-colors"
                >
                  Invalidate current key (system-wide)
                </button>
              ) : (
                <div className="flex items-center gap-3">
                  <span className="text-[10px] text-red-400">This will revoke the key for all clients.</span>
                  <button onClick={handleInvalidate} className="px-3 py-1 bg-red-500 text-white text-[10px] font-bold rounded-lg">Confirm</button>
                  <button onClick={() => setConfirmInvalidate(false)} className="px-3 py-1 bg-surface-secondary text-content-secondary text-[10px] font-bold rounded-lg border border-edge">Cancel</button>
                </div>
              )}
            </div>
          )}
        </div>
      </SectionCard>
    </>
  );
}

/* ======================== DISPLAY ======================== */

function DisplaySection() {
  const [cursorEnabled, setCursorEnabled] = useState(() => localStorage.getItem('cloto-cursor') !== 'off');

  const handleCursorToggle = () => {
    const next = !cursorEnabled;
    setCursorEnabled(next);
    localStorage.setItem('cloto-cursor', next ? 'on' : 'off');
    window.dispatchEvent(new Event('cloto-cursor-toggle'));
  };

  return (
    <SectionCard title="Cursor">
      <div className="space-y-4">
        <Toggle enabled={cursorEnabled} onToggle={handleCursorToggle} label="Custom animated cursor" />
        <p className="text-[10px] text-content-muted">Replaces the native cursor with an animated trail effect using canvas rendering.</p>
      </div>
    </SectionCard>
  );
}

/* ======================== LOG ======================== */

function LogSection() {
  const [logs, setLogs] = useState<string[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const pendingLogs = useRef<string[]>([]);
  const rafId = useRef<number>(0);

  useEventStream(EVENTS_URL, (event) => {
    const timestamp = new Date().toLocaleTimeString();
    const logLine = `[${timestamp}] ${event.type}: ${JSON.stringify(event.data).slice(0, 120)}`;
    pendingLogs.current.push(logLine);
    if (!rafId.current) {
      rafId.current = requestAnimationFrame(() => {
        const batch = pendingLogs.current;
        pendingLogs.current = [];
        rafId.current = 0;
        setLogs(prev => [...prev, ...batch].slice(-100));
      });
    }
  });

  useEffect(() => {
    return () => { if (rafId.current) cancelAnimationFrame(rafId.current); };
  }, []);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [logs]);

  return (
    <SectionCard title="Event Log">
      <div ref={scrollRef} className="h-[60vh] overflow-y-auto font-mono text-[10px] space-y-1 no-scrollbar">
        {logs.length === 0 && <div className="opacity-30">AWAITING_SIGNAL...</div>}
        {logs.map((log, i) => (
          <div key={i} className="text-content-secondary animate-in fade-in slide-in-from-left-1 duration-300">
            <span className="opacity-50 mr-2">&gt;</span>{log}
          </div>
        ))}
      </div>
    </SectionCard>
  );
}

/* ======================== ABOUT ======================== */

function AboutSection() {
  return (
    <>
      <SectionCard title="ClotoCore">
        <div className="space-y-3">
          <p className="text-xs text-content-secondary leading-relaxed">
            AI agent orchestration platform built on a Rust kernel with MCP-based plugin architecture.
          </p>
          <div className="text-2xl font-mono font-black text-brand">v{__APP_VERSION__}</div>
        </div>
      </SectionCard>

      <SectionCard title="License">
        <div className="space-y-2">
          <p className="text-xs text-content-secondary">Business Source License 1.1</p>
          <p className="text-[10px] text-content-muted">Converts to MIT License on 2028-02-14</p>
        </div>
      </SectionCard>

      <SectionCard title="Links">
        <div className="space-y-3">
          {[
            { label: 'Repository', value: 'github.com/Cloto-dev/ClotoCore', href: 'https://github.com/Cloto-dev/ClotoCore' },
            { label: 'Contact', value: 'ClotoCore@proton.me', href: 'mailto:ClotoCore@proton.me' },
          ].map(link => (
            <div key={link.label} className="flex items-center justify-between">
              <span className="text-[10px] text-content-tertiary uppercase tracking-widest font-bold">{link.label}</span>
              <a
                href={link.href}
                target="_blank"
                rel="noopener noreferrer"
                className="text-xs text-brand hover:underline font-mono"
              >
                {link.value}
              </a>
            </div>
          ))}
        </div>
      </SectionCard>
    </>
  );
}
