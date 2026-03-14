import { useState, useEffect, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { api } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';
import { Check, Loader2, Circle, AlertTriangle, Download, Terminal } from 'lucide-react';

type SetupState = 'checking' | 'ready' | 'running' | 'complete' | 'python_missing' | 'error';

interface StepInfo {
  id: string;
  status: 'pending' | 'running' | 'complete' | 'error';
  detail?: string;
  progress?: number;
}

interface LogEntry {
  time: string;
  message: string;
}

const STEP_IDS = [
  'check_python',
  'download',
  'verify',
  'extract',
  'create_venv',
  'install_deps',
  'finalize',
] as const;

export function DependencySetup({ onComplete, debug }: { onComplete: () => void; debug?: boolean }) {
  const { t } = useTranslation('setup');
  const { apiKey } = useApiKey();
  const [state, setState] = useState<SetupState>(debug ? 'ready' : 'checking');
  const [steps, setSteps] = useState<StepInfo[]>(
    STEP_IDS.map(id => ({ id, status: 'pending' }))
  );
  const [pythonGuidance, setPythonGuidance] = useState('');
  const [pythonOs, setPythonOs] = useState('');
  const [errorMessage, setErrorMessage] = useState('');
  const [serverStatuses, setServerStatuses] = useState<{ name: string; status: string }[]>([]);
  const [logEntries, setLogEntries] = useState<LogEntry[]>([]);
  const [showLog, setShowLog] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);
  const logEndRef = useRef<HTMLDivElement>(null);

  const addLog = useCallback((message: string) => {
    const time = new Date().toLocaleTimeString();
    setLogEntries(prev => [...prev, { time, message }]);
  }, []);

  // Auto-scroll log
  useEffect(() => {
    if (showLog && logEndRef.current) {
      logEndRef.current.scrollIntoView({ behavior: 'smooth' });
    }
  }, [logEntries, showLog]);

  // Initial status check (skip in debug mode)
  useEffect(() => {
    if (debug) return;
    api.getSetupStatus()
      .then(s => {
        if (s.setup_complete) {
          setState('complete');
          setTimeout(onComplete, 500);
        } else if (s.setup_in_progress) {
          setState('running');
          connectSSE();
        } else {
          setState('ready');
        }
      })
      .catch(() => setState('ready'));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Cleanup EventSource on unmount
  useEffect(() => {
    return () => {
      eventSourceRef.current?.close();
    };
  }, []);

  const connectSSE = useCallback(() => {
    const url = api.getSetupProgressUrl();
    const es = new EventSource(url);
    eventSourceRef.current = es;

    es.addEventListener('setup', (e: MessageEvent) => {
      try {
        const evt = JSON.parse(e.data);
        addLog(`[${evt.type}] ${evt.step || evt.server_name || ''} ${evt.description || evt.detail || evt.status || evt.error || ''}`);

        switch (evt.type) {
          case 'StepStart':
            setSteps(prev => prev.map(s =>
              s.id === evt.step ? { ...s, status: 'running', detail: evt.description } : s
            ));
            break;
          case 'StepProgress':
            setSteps(prev => prev.map(s =>
              s.id === evt.step ? { ...s, progress: evt.progress, detail: evt.detail } : s
            ));
            break;
          case 'StepComplete':
            setSteps(prev => prev.map(s =>
              s.id === evt.step ? { ...s, status: 'complete', progress: undefined } : s
            ));
            break;
          case 'StepError':
            setSteps(prev => prev.map(s =>
              s.id === evt.step ? { ...s, status: 'error', detail: evt.error } : s
            ));
            setErrorMessage(evt.error);
            setState('error');
            es.close();
            break;
          case 'ServerInstall':
            setServerStatuses(prev => {
              const existing = prev.findIndex(s => s.name === evt.server_name);
              if (existing >= 0) {
                const updated = [...prev];
                updated[existing] = { name: evt.server_name, status: evt.status };
                return updated;
              }
              return [...prev, { name: evt.server_name, status: evt.status }];
            });
            break;
          case 'PythonMissing':
            setPythonOs(evt.os);
            setPythonGuidance(evt.guidance);
            setState('python_missing');
            es.close();
            break;
          case 'Complete':
            setState('complete');
            es.close();
            break;
        }
      } catch {
        // Skip invalid events
      }
    });

    es.onerror = () => {
      // Connection lost — check status
      es.close();
      api.getSetupStatus().then(s => {
        if (s.setup_complete) {
          setState('complete');
        } else if (!s.setup_in_progress) {
          setState('error');
          setErrorMessage('Connection to server lost during setup');
        }
      }).catch(() => {
        setState('error');
        setErrorMessage('Connection to server lost');
      });
    };
  }, [addLog]);

  const handleStart = useCallback(async () => {
    if (!apiKey) return;
    setState('running');
    setSteps(STEP_IDS.map(id => ({ id, status: 'pending' })));
    setServerStatuses([]);
    setErrorMessage('');
    setLogEntries([]);

    connectSSE();

    try {
      await api.startSetup(apiKey);
    } catch (e) {
      setErrorMessage(e instanceof Error ? e.message : 'Failed to start setup');
      setState('error');
    }
  }, [apiKey, connectSSE]);

  const handleRecheck = useCallback(async () => {
    setState('checking');
    try {
      const result = await api.checkPython();
      if (result.available) {
        setState('ready');
      } else {
        setState('python_missing');
      }
    } catch {
      setState('python_missing');
    }
  }, []);

  const stepIcon = (status: StepInfo['status']) => {
    switch (status) {
      case 'complete':
        return <Check size={14} className="text-green-500" />;
      case 'running':
        return <Loader2 size={14} className="text-brand animate-spin" />;
      case 'error':
        return <AlertTriangle size={14} className="text-red-500" />;
      default:
        return <Circle size={14} className="text-content-muted" />;
    }
  };

  const stepLabel = (id: string) => {
    const key = `step_${id}`;
    return t(key, { defaultValue: id });
  };

  return (
    <div className="fixed inset-0 z-50 bg-surface-base flex items-center justify-center select-none">
      <div className="bg-surface-primary border border-edge rounded-2xl shadow-2xl w-full max-w-lg mx-4 overflow-hidden">
        {/* Header */}
        <div className="px-6 pt-6 pb-3">
          <div className="flex items-center gap-2 mb-1">
            <Download size={16} className="text-brand" />
            <h2 className="text-xs font-mono uppercase tracking-widest text-content-primary font-bold">
              {t('title')}
            </h2>
          </div>
          <p className="text-[10px] font-mono text-content-tertiary uppercase tracking-wider">
            {t('subtitle')}
          </p>
        </div>

        {/* Content */}
        <div className="px-6 pb-6">
          {/* Checking state */}
          {state === 'checking' && (
            <div className="flex items-center justify-center py-12">
              <Loader2 size={20} className="text-brand animate-spin mr-3" />
              <span className="text-xs font-mono text-content-secondary">
                {t('checking')}
              </span>
            </div>
          )}

          {/* Python missing state */}
          {state === 'python_missing' && (
            <div className="space-y-4">
              <div className="bg-glass-strong backdrop-blur-sm p-4 rounded-lg border border-edge">
                <div className="flex items-start gap-3">
                  <AlertTriangle size={16} className="text-amber-500 mt-0.5 shrink-0" />
                  <div>
                    <p className="text-xs font-mono font-bold text-content-primary mb-2">
                      {t('python_missing_title')}
                    </p>
                    <p className="text-[10px] font-mono text-content-secondary leading-relaxed mb-3">
                      {t('python_missing_description')}
                    </p>
                    <div className="bg-surface-base rounded p-3 border border-edge">
                      <code className="text-[10px] font-mono text-brand break-all">
                        {pythonGuidance || t(`python_install_${pythonOs || 'linux'}`)}
                      </code>
                    </div>
                  </div>
                </div>
              </div>
              <button
                onClick={handleRecheck}
                className="w-full py-2.5 rounded-lg bg-brand text-white text-xs font-mono font-bold uppercase tracking-wider hover:brightness-110 transition-all"
              >
                {t('python_recheck')}
              </button>
            </div>
          )}

          {/* Ready state */}
          {state === 'ready' && (
            <div className="space-y-4">
              <div className="bg-glass-strong backdrop-blur-sm p-4 rounded-lg border border-edge">
                <p className="text-xs font-mono text-content-secondary leading-relaxed">
                  {t('start_description')}
                </p>
              </div>
              <button
                onClick={handleStart}
                disabled={!apiKey}
                className="w-full py-2.5 rounded-lg bg-brand text-white text-xs font-mono font-bold uppercase tracking-wider hover:brightness-110 transition-all disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {t('start_setup')}
              </button>
            </div>
          )}

          {/* Running state */}
          {state === 'running' && (
            <div className="space-y-4">
              {/* Step list */}
              <div className="space-y-2">
                {steps.map(step => (
                  <div
                    key={step.id}
                    className={`flex items-center gap-3 px-3 py-2 rounded-lg transition-colors ${
                      step.status === 'running'
                        ? 'bg-glass-strong border border-brand/30'
                        : step.status === 'complete'
                        ? 'bg-glass border border-edge'
                        : 'border border-transparent'
                    }`}
                  >
                    {stepIcon(step.status)}
                    <span className={`text-[11px] font-mono flex-1 ${
                      step.status === 'running' ? 'text-content-primary font-bold' :
                      step.status === 'complete' ? 'text-content-secondary' :
                      'text-content-muted'
                    }`}>
                      {stepLabel(step.id)}
                    </span>
                    {step.progress !== undefined && (
                      <span className="text-[9px] font-mono text-content-tertiary">
                        {step.detail || `${Math.round(step.progress * 100)}%`}
                      </span>
                    )}
                  </div>
                ))}
              </div>

              {/* Download progress bar */}
              {steps.find(s => s.id === 'download' && s.status === 'running' && s.progress !== undefined) && (
                <div className="h-1 bg-glass rounded-full overflow-hidden">
                  <div
                    className="h-full bg-brand transition-all duration-300"
                    style={{ width: `${(steps.find(s => s.id === 'download')?.progress ?? 0) * 100}%` }}
                  />
                </div>
              )}

              {/* Per-server install status */}
              {serverStatuses.length > 0 && (
                <div className="bg-glass-strong backdrop-blur-sm rounded-lg border border-edge p-3 max-h-32 overflow-y-auto">
                  <div className="space-y-1">
                    {serverStatuses.map(s => (
                      <div key={s.name} className="flex items-center justify-between">
                        <span className="text-[10px] font-mono text-content-secondary">{s.name}</span>
                        <span className={`text-[9px] font-mono ${
                          s.status === 'installed' ? 'text-green-500' :
                          s.status === 'failed' ? 'text-red-500' :
                          'text-brand'
                        }`}>
                          {s.status === 'installing' ? <Loader2 size={10} className="animate-spin inline" /> : s.status}
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* Collapsible log */}
              <button
                onClick={() => setShowLog(v => !v)}
                className="flex items-center gap-1.5 text-[9px] font-mono text-content-tertiary hover:text-content-secondary transition-colors"
              >
                <Terminal size={10} />
                {showLog ? t('hide_log') : t('show_log')}
              </button>
              {showLog && (
                <div className="bg-surface-base rounded-lg border border-edge p-3 max-h-40 overflow-y-auto font-mono text-[9px] text-content-tertiary">
                  {logEntries.map((entry, i) => (
                    <div key={i} className="flex gap-2">
                      <span className="text-content-muted shrink-0">{entry.time}</span>
                      <span>{entry.message}</span>
                    </div>
                  ))}
                  <div ref={logEndRef} />
                </div>
              )}
            </div>
          )}

          {/* Error state */}
          {state === 'error' && (
            <div className="space-y-4">
              <div className="bg-glass-strong backdrop-blur-sm p-4 rounded-lg border border-red-500/30">
                <div className="flex items-start gap-3">
                  <AlertTriangle size={16} className="text-red-500 mt-0.5 shrink-0" />
                  <div>
                    <p className="text-xs font-mono font-bold text-content-primary mb-2">
                      {t('error_title')}
                    </p>
                    <p className="text-[10px] font-mono text-content-secondary leading-relaxed mb-2">
                      {t('error_description')}
                    </p>
                    {errorMessage && (
                      <div className="bg-surface-base rounded p-2 border border-edge">
                        <code className="text-[9px] font-mono text-red-400 break-all">
                          {errorMessage}
                        </code>
                      </div>
                    )}
                  </div>
                </div>
              </div>
              <button
                onClick={handleStart}
                disabled={!apiKey}
                className="w-full py-2.5 rounded-lg bg-brand text-white text-xs font-mono font-bold uppercase tracking-wider hover:brightness-110 transition-all disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {t('retry')}
              </button>
            </div>
          )}

          {/* Complete state */}
          {state === 'complete' && (
            <div className="space-y-4">
              <div className="bg-glass-strong backdrop-blur-sm p-4 rounded-lg border border-green-500/30">
                <div className="flex items-start gap-3">
                  <Check size={16} className="text-green-500 mt-0.5 shrink-0" />
                  <div>
                    <p className="text-xs font-mono font-bold text-content-primary mb-1">
                      {t('complete_title')}
                    </p>
                    <p className="text-[10px] font-mono text-content-secondary">
                      {t('complete_description')}
                    </p>
                  </div>
                </div>
              </div>
              <button
                onClick={onComplete}
                className="w-full py-2.5 rounded-lg bg-brand text-white text-xs font-mono font-bold uppercase tracking-wider hover:brightness-110 transition-all"
              >
                {t('continue')}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
