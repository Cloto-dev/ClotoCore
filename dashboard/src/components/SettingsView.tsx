import { useState, useEffect, useRef } from 'react';
import { Sun, Moon, Monitor, Eye, EyeOff, MousePointer, ScrollText, Info, Shield, AlertTriangle, Zap, Settings } from 'lucide-react';
import { ViewHeader } from './ViewHeader';
import { useTheme } from '../hooks/useTheme';
import { useApiKey } from '../contexts/ApiKeyContext';
import { useEventStream } from '../hooks/useEventStream';
import { EVENTS_URL, api } from '../services/api';

type Section = 'general' | 'security' | 'display' | 'advanced' | 'log' | 'about';

const NAV_ITEMS: { id: Section; label: string; icon: typeof Sun }[] = [
  { id: 'general', label: 'GENERAL', icon: Sun },
  { id: 'security', label: 'SECURITY', icon: Shield },
  { id: 'display', label: 'DISPLAY', icon: MousePointer },
  { id: 'advanced', label: 'ADVANCED', icon: Zap },
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

function SectionCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="bg-glass-strong backdrop-blur-sm p-5 rounded-lg border border-edge shadow-sm mb-4">
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
      await api.listCronJobs(newKey.trim());
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

      <LlmProvidersSection />
    </>
  );
}

/* ======================== LLM PROVIDERS ======================== */

function LlmProvidersSection() {
  const { apiKey } = useApiKey();
  const [providers, setProviders] = useState<Array<{ id: string; display_name: string; has_key: boolean; model_id: string }>>([]);
  const [keyInputs, setKeyInputs] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState<string | null>(null);

  useEffect(() => {
    if (!apiKey) return;
    api.listLlmProviders(apiKey).then(d => setProviders(d.providers)).catch(() => {});
  }, [apiKey]);

  const handleSave = async (providerId: string) => {
    if (!apiKey || !keyInputs[providerId]?.trim()) return;
    setSaving(providerId);
    try {
      await api.setLlmProviderKey(providerId, apiKey, keyInputs[providerId].trim());
      setKeyInputs(prev => ({ ...prev, [providerId]: '' }));
      const d = await api.listLlmProviders(apiKey);
      setProviders(d.providers);
    } catch { /* ignore */ }
    setSaving(null);
  };

  const handleDelete = async (providerId: string) => {
    if (!apiKey) return;
    await api.deleteLlmProviderKey(providerId, apiKey);
    const d = await api.listLlmProviders(apiKey);
    setProviders(d.providers);
  };

  return (
    <SectionCard title="LLM Providers">
      <p className="text-[10px] text-content-muted mb-4">API keys are held by the kernel and never exposed to MCP servers (MGP §13.4).</p>
      <div className="space-y-3">
        {providers.map(p => (
          <div key={p.id} className="flex items-center gap-3 p-3 bg-surface-secondary rounded-lg border border-edge-subtle">
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <span className={`w-2 h-2 rounded-full ${p.has_key ? 'bg-green-500' : 'bg-amber-500'}`} />
                <span className="text-xs font-bold text-content-primary">{p.display_name}</span>
                <span className="text-[9px] font-mono text-content-muted">{p.model_id}</span>
              </div>
              <div className="flex gap-2 mt-2">
                <input
                  type="password"
                  value={keyInputs[p.id] || ''}
                  onChange={e => setKeyInputs(prev => ({ ...prev, [p.id]: e.target.value }))}
                  placeholder={p.has_key ? '••••••• (saved)' : 'Enter API key'}
                  className="flex-1 bg-surface-base border border-edge rounded px-2 py-1 text-[10px] font-mono text-content-primary placeholder:text-content-muted"
                />
                <button
                  onClick={() => handleSave(p.id)}
                  disabled={!keyInputs[p.id]?.trim() || saving === p.id}
                  className="px-3 py-1 bg-brand text-white text-[10px] font-bold rounded disabled:opacity-40"
                >
                  {saving === p.id ? '...' : 'Save'}
                </button>
                {p.has_key && (
                  <button
                    onClick={() => handleDelete(p.id)}
                    className="px-2 py-1 text-red-400 text-[10px] hover:bg-red-500/10 rounded"
                  >
                    Clear
                  </button>
                )}
              </div>
            </div>
          </div>
        ))}
        {providers.length === 0 && (
          <p className="text-[10px] text-content-muted italic">No providers configured.</p>
        )}
      </div>
    </SectionCard>
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

/* ======================== ADVANCED ======================== */

function AdvancedSection() {
  const { apiKey } = useApiKey();
  const [yoloEnabled, setYoloEnabled] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api.fetchJson<{ enabled: boolean }>('/settings/yolo', apiKey)
      .then(data => setYoloEnabled(data.enabled))
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [apiKey]);

  const handleToggle = async () => {
    const next = !yoloEnabled;
    try {
      await api.put('/settings/yolo', { enabled: next }, apiKey);
      setYoloEnabled(next);
    } catch (err) {
      console.error('Failed to toggle YOLO mode:', err);
    }
  };

  return (
    <>
      <SectionCard title="YOLO Mode">
        <div className="space-y-4">
          {!loading && (
            <Toggle enabled={yoloEnabled} onToggle={handleToggle} label="Auto-approve MCP permissions" />
          )}
          {yoloEnabled && (
            <div className="flex items-start gap-2 p-3 rounded-lg bg-amber-500/10 border border-amber-500/30">
              <AlertTriangle size={14} className="text-amber-400 mt-0.5 shrink-0" />
              <div className="space-y-1">
                <p className="text-[10px] font-bold text-amber-400 uppercase tracking-widest">Warning</p>
                <p className="text-[10px] text-content-muted">MCP server permissions are auto-approved without manual review. SafetyGate and code validation remain active.</p>
              </div>
            </div>
          )}
          {!yoloEnabled && (
            <p className="text-[10px] text-content-muted">When enabled, MCP server permission requests are automatically approved. SafetyGate post-validation remains active as a safety net.</p>
          )}
        </div>
      </SectionCard>
    </>
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
