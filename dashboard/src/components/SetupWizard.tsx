import {
  AlertTriangle,
  Brain,
  Check,
  ChevronDown,
  Circle,
  Clock,
  Loader2,
  Monitor,
  Moon,
  Server,
  Settings,
  Sun,
  Users,
} from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApiKey } from '../contexts/ApiKeyContext';
import { useUserIdentity } from '../contexts/UserIdentityContext';
import { useApi } from '../hooks/useApi';
import { useTheme } from '../hooks/useTheme';
import { getCustomLanguages } from '../i18n';
import { getAutoApiKey } from '../lib/tauri';
import { createAuthenticatedApi } from '../services/api';

const BUILTIN_LANGUAGES = [
  { code: 'en', label: 'English' },
  { code: 'ja', label: '日本語' },
];

const TOTAL_STEPS = 7;

// ============================================================
// Preset Definitions
// ============================================================

import { SERVER_PRESETS, STANDARD_SERVERS } from '../lib/presets';

const ENGINE_IDS = ['mind.cerebras', 'mind.deepseek', 'mind.claude', 'mind.ollama'] as const;

const ALL_SELECTABLE_SERVER_IDS = [
  'memory.cpersona',
  'tool.terminal',
  'tool.cron',
  'tool.websearch',
  'tool.research',
  'tool.agent_utils',
  'tool.embedding',
  'tool.imagegen',
  'vision.capture',
  'vision.gaze_webcam',
  'voice.stt',
  'voice.tts',
] as const;

/** Map server ID → translation key (e.g., "memory.cpersona" → "server_memory_cpersona") */
function serverTKey(id: string): string {
  return `server_${id.replace('.', '_')}`;
}

/** Map engine ID → translation key (e.g., "mind.cerebras" → "engine_cerebras") */
function engineTKey(id: string): string {
  return `engine_${id.replace('mind.', '')}`;
}

const MANUAL_START_SERVERS = new Set([
  'vision.gaze_webcam',
  'vision.capture',
  'tool.imagegen',
  'voice.stt',
  'voice.tts',
]);

const DEFAULT_AGENT_ID = 'agent.cloto_default';

// ============================================================
// Component
// ============================================================

interface Props {
  onComplete: () => void;
}

export function SetupWizard({ onComplete }: Props) {
  const [step, setStep] = useState(0);
  const { t, i18n } = useTranslation('wizard');
  const { preference, setPreference } = useTheme();
  const { identity, setIdentity } = useUserIdentity();
  const { apiKey } = useApiKey();
  const [customLangs, setCustomLangs] = useState<{ code: string; label: string }[]>([]);
  const [displayName, setDisplayName] = useState(identity.name === 'User' ? '' : identity.name);

  // Preset state
  const [selectedPreset, setSelectedPreset] = useState<string>('advanced');
  const [selectedEngine, setSelectedEngine] = useState('mind.deepseek');
  const [customServers, setCustomServers] = useState<Set<string>>(new Set(STANDARD_SERVERS));
  const [applying, setApplying] = useState(false);

  // Installation state (Step 5)
  const api = useApi();
  const [installStarted, setInstallStarted] = useState(false);
  const [installComplete, setInstallComplete] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);
  const [serverStatuses, setServerStatuses] = useState<Array<{ name: string; status: string }>>([]);
  const [installSteps, setInstallSteps] = useState<
    Array<{ step: string; description: string; status: string; detail?: string; progress?: number }>
  >([]);
  const eventSourceRef = useRef<EventSource | null>(null);

  useEffect(() => {
    return () => {
      eventSourceRef.current?.close();
    };
  }, []);

  useEffect(() => {
    getCustomLanguages().then(setCustomLangs);
  }, []);

  const builtinCodes = new Set(BUILTIN_LANGUAGES.map((l) => l.code));
  const allLanguages = [...BUILTIN_LANGUAGES, ...customLangs.filter((l) => !builtinCodes.has(l.code))];

  const next = () => setStep((s) => Math.min(s + 1, TOTAL_STEPS - 1));
  // Skip step 5 (installation) when going back — it's a one-way step
  const back = () =>
    setStep((s) => {
      const prev = Math.max(s - 1, 0);
      return prev === 5 ? 4 : prev;
    });

  // When preset changes, sync engine and custom servers
  const handlePresetSelect = (presetId: string) => {
    setSelectedPreset(presetId);
    if (presetId !== 'custom') {
      const preset = SERVER_PRESETS.find((p) => p.id === presetId);
      if (preset) {
        setSelectedEngine(preset.defaultEngine);
        setCustomServers(new Set(preset.servers));
      }
    }
  };

  const toggleCustomServer = (serverId: string) => {
    setCustomServers((prev) => {
      const next = new Set(prev);
      if (next.has(serverId)) next.delete(serverId);
      else next.add(serverId);
      return next;
    });
  };

  const getActiveServers = (): string[] => {
    const base =
      selectedPreset === 'custom'
        ? Array.from(customServers)
        : (SERVER_PRESETS.find((p) => p.id === selectedPreset)?.servers ?? STANDARD_SERVERS);
    // Always include the selected engine in server grants
    if (selectedEngine && !base.includes(selectedEngine)) {
      return [...base, selectedEngine];
    }
    return base;
  };

  // Apply preset to backend
  const applyPreset = async () => {
    // Resolve API key: context → sessionStorage → direct Tauri invoke
    let key = apiKey || sessionStorage.getItem('cloto-api-key') || '';
    if (!key) {
      const tauriKey = await getAutoApiKey();
      if (tauriKey) {
        key = tauriKey;
        sessionStorage.setItem('cloto-api-key', key);
      }
    }
    if (!key) {
      console.warn('[SetupWizard] API key not available, skipping preset apply');
      return;
    }
    setApplying(true);
    try {
      const authedApi = createAuthenticatedApi(key);
      const servers = getActiveServers();

      // Update engine (omit metadata to preserve existing values)
      await authedApi.updateAgent(DEFAULT_AGENT_ID, {
        default_engine_id: selectedEngine,
      });

      // Update server grants: PUT each server with a server_grant entry
      // First get current grants, then compute diff
      const currentAccess = await authedApi.getAgentAccess(DEFAULT_AGENT_ID);
      const currentGranted = new Set(
        currentAccess.entries
          .filter((e) => e.entry_type === 'server_grant' && e.permission === 'allow')
          .map((e) => e.server_id),
      );

      const desired = new Set(servers);

      // Add new grants
      for (const serverId of desired) {
        if (!currentGranted.has(serverId)) {
          await authedApi.putMcpServerAccess(serverId, [
            {
              entry_type: 'server_grant',
              agent_id: DEFAULT_AGENT_ID,
              server_id: serverId,
              permission: 'allow',
              granted_at: new Date().toISOString(),
            },
          ]);
        }
      }

      // Remove revoked grants
      for (const serverId of currentGranted) {
        if (!desired.has(serverId)) {
          await authedApi.putMcpServerAccess(serverId, []);
        }
      }
    } catch (e) {
      console.error('Failed to apply preset:', e);
    } finally {
      setApplying(false);
    }
  };

  const handleFinish = () => {
    if (displayName.trim()) {
      setIdentity(identity.id, displayName.trim());
    }
    onComplete();
  };

  const handleNameBlur = () => {
    if (displayName.trim()) {
      setIdentity(identity.id, displayName.trim());
    }
  };

  // Step 4: apply preset then advance
  const handlePresetNext = async () => {
    await applyPreset();
    next();
  };

  // Step 4: skip — jump directly to Quick Guide (step 6), skipping installation
  const handlePresetSkip = () => {
    setStep(6);
  };

  // Step 5: Start batch installation
  const startInstallation = useCallback(async () => {
    if (installStarted) return;
    setInstallStarted(true);
    setInstallError(null);
    setServerStatuses([]);
    setInstallSteps([]);

    try {
      const servers = getActiveServers();
      await api.batchInstallMarketplaceServers({ server_ids: servers, auto_start: true });

      // Connect SSE for progress
      const progressUrl = api.getMarketplaceProgressUrl();
      const sseUrl = `${progressUrl}?api_key=${encodeURIComponent(api.apiKey || '')}`;
      const es = new EventSource(sseUrl);
      eventSourceRef.current = es;

      es.addEventListener('setup', (ev: MessageEvent) => {
        try {
          const data = JSON.parse(ev.data);
          switch (data.type) {
            case 'StepStart':
              setInstallSteps((prev) => [
                ...prev,
                { step: data.step, description: data.description, status: 'running' },
              ]);
              break;
            case 'StepProgress':
              setInstallSteps((prev) =>
                prev.map((s) => (s.step === data.step ? { ...s, progress: data.progress, detail: data.detail } : s)),
              );
              break;
            case 'StepComplete':
              setInstallSteps((prev) => prev.map((s) => (s.step === data.step ? { ...s, status: 'complete' } : s)));
              break;
            case 'StepError':
              setInstallSteps((prev) => prev.map((s) => (s.step === data.step ? { ...s, status: 'error' } : s)));
              if (!data.recoverable) {
                setInstallError(data.error);
                es.close();
              }
              break;
            case 'ServerInstall':
              setServerStatuses((prev) => {
                const existing = prev.findIndex((s) => s.name === data.server_name);
                if (existing >= 0) {
                  const copy = [...prev];
                  copy[existing] = { name: data.server_name, status: data.status };
                  return copy;
                }
                return [...prev, { name: data.server_name, status: data.status }];
              });
              break;
            case 'Complete':
              setInstallComplete(true);
              es.close();
              // Auto-advance to next step after brief delay
              setTimeout(() => next(), 1000);
              break;
          }
        } catch {
          /* ignore parse errors */
        }
      });

      es.onerror = () => {
        es.close();
        if (!installComplete) {
          setInstallComplete(true);
          setTimeout(() => next(), 1000);
        }
      };
    } catch (e) {
      setInstallError(e instanceof Error ? e.message : 'Installation failed');
    }
  }, [installStarted, api, installComplete, getActiveServers, next]);

  // Auto-start installation when entering step 5
  useEffect(() => {
    if (step === 5 && !installStarted) {
      startInstallation();
    }
  }, [step, installStarted, startInstallation]);

  const themes = [
    { value: 'light' as const, icon: Sun, label: t('theme_light') },
    { value: 'dark' as const, icon: Moon, label: t('theme_dark') },
    { value: 'system' as const, icon: Monitor, label: t('theme_system') },
  ];

  const guideItems = [
    { icon: Users, label: 'Agent', desc: t('guide_agents') },
    { icon: Server, label: 'MCP', desc: t('guide_mcp') },
    { icon: Clock, label: 'Cron', desc: t('guide_cron') },
    { icon: Brain, label: 'Memory', desc: t('guide_memory') },
    { icon: Settings, label: 'Settings', desc: t('guide_settings') },
  ];

  return (
    <div className="fixed inset-0 z-50 bg-surface-base flex items-center justify-center">
      <div className="bg-surface-primary border border-edge rounded-2xl shadow-2xl w-full max-w-lg mx-4 flex flex-col">
        {/* Content */}
        <div className="p-8 min-h-[340px] flex flex-col items-center justify-center">
          {step === 0 && (
            <div className="text-center space-y-6">
              <h1 className="text-3xl font-black tracking-[0.15em] text-content-primary">CLOTO SYSTEM</h1>
              <p className="text-sm text-content-secondary max-w-sm">{t('welcome_desc')}</p>
              <button
                onClick={next}
                className="px-8 py-3 bg-brand text-white rounded-xl text-sm font-bold hover:opacity-90 transition-opacity"
              >
                {t('get_started')}
              </button>
            </div>
          )}

          {step === 1 && (
            <div className="text-center space-y-6 w-full max-w-xs">
              <h2 className="text-xl font-bold text-content-primary">{t('select_language')}</h2>
              <select
                value={i18n.language.split('-')[0]}
                onChange={(e) => i18n.changeLanguage(e.target.value)}
                className="w-full px-4 py-3 bg-surface-secondary border border-edge rounded-xl text-sm text-content-primary focus:border-brand focus:outline-none transition-colors"
              >
                {allLanguages.map((lang) => (
                  <option key={lang.code} value={lang.code}>
                    {lang.label}
                  </option>
                ))}
              </select>
            </div>
          )}

          {step === 2 && (
            <div className="text-center space-y-6">
              <h2 className="text-xl font-bold text-content-primary">{t('select_theme')}</h2>
              <div className="flex gap-3">
                {themes.map(({ value, icon: Icon, label }) => (
                  <button
                    key={value}
                    onClick={() => setPreference(value)}
                    className={`flex items-center gap-2 px-5 py-3 rounded-xl text-sm font-bold transition-all ${
                      preference === value
                        ? 'bg-brand text-white shadow-md'
                        : 'bg-surface-secondary text-content-secondary hover:text-content-primary border border-edge hover:border-brand'
                    }`}
                  >
                    <Icon size={16} />
                    {label}
                  </button>
                ))}
              </div>
            </div>
          )}

          {step === 3 && (
            <div className="text-center space-y-6 w-full max-w-xs">
              <h2 className="text-xl font-bold text-content-primary">{t('enter_name')}</h2>
              <input
                type="text"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                onBlur={handleNameBlur}
                placeholder={t('name_placeholder')}
                className="w-full px-4 py-3 bg-surface-secondary border border-edge rounded-xl text-sm text-content-primary focus:border-brand focus:outline-none transition-colors text-center"
              />
              <p className="text-[11px] text-content-tertiary">{t('name_hint')}</p>
            </div>
          )}

          {step === 4 && (
            <PresetStep
              t={t}
              selectedPreset={selectedPreset}
              selectedEngine={selectedEngine}
              customServers={customServers}
              onSelectPreset={handlePresetSelect}
              onSelectEngine={setSelectedEngine}
              onToggleServer={toggleCustomServer}
            />
          )}

          {step === 5 && (
            <div className="space-y-4 w-full">
              <h2 className="text-xl font-bold text-content-primary text-center">
                {t('step_install', { defaultValue: 'Installing Servers' })}
              </h2>

              {/* Progress steps */}
              {installSteps.length > 0 && (
                <div className="space-y-2">
                  {installSteps.map((s) => (
                    <div key={s.step} className="flex items-center gap-2 text-[11px] font-mono">
                      {s.status === 'running' && <Loader2 size={12} className="text-brand animate-spin shrink-0" />}
                      {s.status === 'complete' && <Check size={12} className="text-emerald-500 shrink-0" />}
                      {s.status === 'error' && <AlertTriangle size={12} className="text-red-500 shrink-0" />}
                      <span className="text-content-secondary">{s.description}</span>
                      {s.detail && <span className="text-content-tertiary ml-auto">{s.detail}</span>}
                    </div>
                  ))}
                </div>
              )}

              {/* Server statuses */}
              {serverStatuses.length > 0 && (
                <div className="max-h-[140px] overflow-y-auto space-y-1 border border-edge rounded-lg p-2">
                  {serverStatuses.map((s) => (
                    <div key={s.name} className="flex items-center gap-2 text-[11px]">
                      {s.status === 'installing' && <Loader2 size={10} className="text-brand animate-spin shrink-0" />}
                      {s.status === 'installed' && <Check size={10} className="text-emerald-500 shrink-0" />}
                      {s.status === 'failed' && <AlertTriangle size={10} className="text-red-500 shrink-0" />}
                      {s.status === 'skipped' && <Circle size={10} className="text-content-tertiary shrink-0" />}
                      <span
                        className={`font-sans ${s.status === 'failed' ? 'text-red-400' : s.status === 'skipped' ? 'text-content-tertiary' : 'text-content-secondary'}`}
                      >
                        {s.name}
                      </span>
                      {s.status === 'skipped' && (
                        <span className="text-[9px] text-content-tertiary ml-auto">
                          {t('step_install_skipped', { defaultValue: 'already installed' })}
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              )}

              {/* Error */}
              {installError && (
                <div className="bg-red-500/10 border border-red-500/30 rounded-lg p-3 text-[11px] text-red-400">
                  {installError}
                </div>
              )}

              {/* Completion */}
              {installComplete && !installError && (
                <div className="text-center text-[11px] text-emerald-500 font-sans">
                  {t('step_install_complete', { defaultValue: 'All servers installed' })}
                </div>
              )}

              {/* Waiting state */}
              {!installStarted && !installComplete && (
                <div className="text-center text-[11px] text-content-tertiary">
                  {t('step_install_preparing', { defaultValue: 'Preparing installation...' })}
                </div>
              )}
            </div>
          )}

          {step === 6 && (
            <div className="space-y-5 w-full">
              <h2 className="text-xl font-bold text-content-primary text-center">{t('quick_guide')}</h2>
              <div className="space-y-3">
                {guideItems.map(({ icon: Icon, label, desc }) => (
                  <div
                    key={label}
                    className="flex items-start gap-3 px-4 py-3 bg-surface-secondary rounded-xl border border-edge"
                  >
                    <Icon size={18} className="text-brand shrink-0 mt-0.5" />
                    <div>
                      <span className="text-xs font-bold text-content-primary">{label}</span>
                      <p className="text-[11px] text-content-secondary mt-0.5">{desc}</p>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Footer: dots + nav buttons */}
        <div className="px-8 pb-6 flex items-center justify-between">
          {/* Back button (hidden during installation step) */}
          <div className="w-20">
            {step > 0 && step < TOTAL_STEPS && step !== 5 && (
              <button
                onClick={back}
                className="text-xs font-bold text-content-tertiary hover:text-content-primary transition-colors"
              >
                {t('back')}
              </button>
            )}
          </div>

          {/* Step dots */}
          <div className="flex gap-2">
            {Array.from({ length: TOTAL_STEPS }, (_, i) => (
              <div
                key={i}
                className={`w-2 h-2 rounded-full transition-colors ${i === step ? 'bg-brand' : 'bg-edge'}`}
              />
            ))}
          </div>

          {/* Next / Skip / Finish button */}
          <div className="w-20 flex justify-end">
            {step === 0 ? (
              <div /> // Welcome has its own CTA
            ) : step === 4 ? (
              <div className="flex items-center gap-2">
                <button
                  onClick={handlePresetSkip}
                  className="text-[11px] text-content-tertiary hover:text-content-primary transition-colors"
                >
                  {t('preset_skip')}
                </button>
                <button
                  onClick={handlePresetNext}
                  disabled={applying}
                  className="px-4 py-2 bg-brand text-white rounded-lg text-xs font-bold hover:opacity-90 transition-opacity disabled:opacity-50"
                >
                  {applying ? '...' : t('next')}
                </button>
              </div>
            ) : step === 5 ? (
              <div /> // Installation auto-advances
            ) : step < TOTAL_STEPS - 1 ? (
              <button
                onClick={next}
                className="px-4 py-2 bg-brand text-white rounded-lg text-xs font-bold hover:opacity-90 transition-opacity"
              >
                {t('next')}
              </button>
            ) : (
              <button
                onClick={handleFinish}
                className="px-4 py-2 bg-brand text-white rounded-lg text-xs font-bold hover:opacity-90 transition-opacity whitespace-nowrap"
              >
                {t('finish')}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ============================================================
// Preset Step Sub-component
// ============================================================

interface PresetStepProps {
  t: (key: string) => string;
  selectedPreset: string;
  selectedEngine: string;
  customServers: Set<string>;
  onSelectPreset: (id: string) => void;
  onSelectEngine: (id: string) => void;
  onToggleServer: (id: string) => void;
}

function PresetStep({
  t,
  selectedPreset,
  selectedEngine,
  customServers,
  onSelectPreset,
  onSelectEngine,
  onToggleServer,
}: PresetStepProps) {
  const presetCards = [...SERVER_PRESETS.map((p) => ({ id: p.id, icon: p.icon })), { id: 'custom', icon: Settings }];

  const activeServers =
    selectedPreset === 'custom'
      ? customServers
      : new Set(SERVER_PRESETS.find((p) => p.id === selectedPreset)?.servers ?? STANDARD_SERVERS);

  const hasManualStart = Array.from(activeServers).some((s) => MANUAL_START_SERVERS.has(s));

  return (
    <div className="space-y-4 w-full">
      <div className="text-center">
        <h2 className="text-xl font-bold text-content-primary">{t('preset_title')}</h2>
        <p className="text-[11px] text-content-tertiary mt-1">{t('preset_desc')}</p>
      </div>

      {/* Preset Cards */}
      <div className="grid grid-cols-5 gap-2">
        {presetCards.map(({ id, icon: Icon }) => (
          <button
            key={id}
            onClick={() => onSelectPreset(id)}
            className={`flex flex-col items-center gap-1.5 p-3 rounded-xl text-center transition-all ${
              selectedPreset === id
                ? 'bg-brand text-white shadow-md'
                : 'bg-surface-secondary text-content-secondary hover:text-content-primary border border-edge hover:border-brand'
            }`}
          >
            <Icon size={18} />
            <span className="text-[10px] font-bold leading-tight">{t(`preset_${id}`)}</span>
          </button>
        ))}
      </div>

      {/* Description */}
      <p className="text-[11px] text-content-secondary text-center px-4">{t(`preset_${selectedPreset}_desc`)}</p>

      {/* Engine selector */}
      <div className="space-y-1.5">
        <label className="text-[10px] font-bold text-content-tertiary uppercase tracking-wider">
          {t('preset_engine')}
        </label>
        <div className="relative">
          <select
            value={selectedEngine}
            onChange={(e) => onSelectEngine(e.target.value)}
            className="w-full px-3 py-2 bg-surface-secondary border border-edge rounded-lg text-xs text-content-primary focus:border-brand focus:outline-none appearance-none"
          >
            {ENGINE_IDS.map((id) => (
              <option key={id} value={id}>
                {t(engineTKey(id))}
              </option>
            ))}
          </select>
          <ChevronDown
            size={12}
            className="absolute right-3 top-1/2 -translate-y-1/2 text-content-tertiary pointer-events-none"
          />
        </div>
      </div>

      {/* Server list / Custom checkboxes */}
      <div className="space-y-1.5">
        <label className="text-[10px] font-bold text-content-tertiary uppercase tracking-wider">
          {t('preset_servers')}
        </label>
        {selectedPreset === 'custom' ? (
          <div className="grid grid-cols-2 gap-1.5 max-h-[120px] overflow-y-auto pr-1">
            {ALL_SELECTABLE_SERVER_IDS.map((id) => (
              <label
                key={id}
                className={`flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[11px] cursor-pointer transition-colors ${
                  customServers.has(id)
                    ? 'bg-brand/10 text-content-primary border border-brand/30'
                    : 'bg-surface-secondary text-content-tertiary border border-transparent hover:border-edge'
                }`}
              >
                <input
                  type="checkbox"
                  checked={customServers.has(id)}
                  onChange={() => onToggleServer(id)}
                  className="sr-only"
                />
                <div
                  className={`w-3 h-3 rounded border flex items-center justify-center shrink-0 ${
                    customServers.has(id) ? 'bg-brand border-brand' : 'border-edge'
                  }`}
                >
                  {customServers.has(id) && (
                    <svg width="8" height="8" viewBox="0 0 8 8" fill="none">
                      <path
                        d="M1.5 4L3 5.5L6.5 2"
                        stroke="white"
                        strokeWidth="1.5"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      />
                    </svg>
                  )}
                </div>
                <span className="truncate">{t(serverTKey(id))}</span>
                {MANUAL_START_SERVERS.has(id) && <span className="text-[9px] text-amber-500 shrink-0">*</span>}
              </label>
            ))}
          </div>
        ) : (
          <div className="flex flex-wrap gap-1.5">
            {Array.from(activeServers).map((id) => (
              <span
                key={id}
                className="px-2 py-1 bg-surface-secondary border border-edge rounded-md text-[10px] text-content-secondary"
              >
                {t(serverTKey(id))}
                {MANUAL_START_SERVERS.has(id) && <span className="text-amber-500 ml-0.5">*</span>}
              </span>
            ))}
          </div>
        )}
        {hasManualStart && <p className="text-[9px] text-amber-500">* {t('preset_manual_note')}</p>}
      </div>
    </div>
  );
}
