import { AlertTriangle, CheckCircle, Loader, Package } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import type { MarketplaceCatalogEntry } from '../../types';
import { Modal } from '../Modal';

interface InstallDialogProps {
  entry: MarketplaceCatalogEntry;
  onClose: () => void;
  onInstalled: () => void;
}

interface ProgressStep {
  step: string;
  description?: string;
  progress?: number;
  detail?: string;
  status: 'pending' | 'running' | 'complete' | 'error';
  error?: string;
}

export function InstallDialog({ entry, onClose, onInstalled }: InstallDialogProps) {
  const { t } = useTranslation('mcp');
  const api = useApi();

  // Env vars form state -- initialize from entry.env_vars defaults
  const [envVars, setEnvVars] = useState<Record<string, string>>(() => {
    const vars: Record<string, string> = {};
    for (const ev of entry.env_vars) {
      vars[ev.key] = ev.default ?? '';
    }
    for (const ev of entry.optional_env_vars) {
      vars[ev.key] = ev.default ?? '';
    }
    return vars;
  });

  const [installing, setInstalling] = useState(false);
  const [steps, setSteps] = useState<ProgressStep[]>([]);
  const [complete, setComplete] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  // Cleanup EventSource on unmount
  useEffect(() => {
    return () => {
      if (eventSourceRef.current) {
        eventSourceRef.current.close();
      }
    };
  }, []);

  function handleEnvChange(key: string, value: string) {
    setEnvVars((prev) => ({ ...prev, [key]: value }));
  }

  async function handleInstall() {
    setInstalling(true);
    setInstallError(null);
    setSteps([]);
    setComplete(false);

    try {
      // Start installation
      await api.installMarketplaceServer({
        server_id: entry.id,
        env: envVars,
        auto_start: true,
      });

      // Connect to SSE progress stream
      const progressUrl = api.getMarketplaceProgressUrl();
      const url = `${progressUrl}?server_id=${encodeURIComponent(entry.id)}&api_key=${encodeURIComponent(api.apiKey)}`;
      const es = new EventSource(url);
      eventSourceRef.current = es;

      es.addEventListener('setup', (event: MessageEvent) => {
        try {
          const data = JSON.parse(event.data);
          handleProgressEvent(data);
        } catch {
          // ignore parse errors
        }
      });

      es.onerror = () => {
        es.close();
        eventSourceRef.current = null;
        if (!complete) {
          // If we haven't received a Complete event, treat as success
          // (the install API call already succeeded)
          setComplete(true);
          setInstalling(false);
          onInstalled();
        }
      };
    } catch (err) {
      setInstallError(err instanceof Error ? err.message : t('marketplace.install_error'));
      setInstalling(false);
    }
  }

  function handleProgressEvent(data: Record<string, unknown>) {
    const type = data.type as string;

    switch (type) {
      case 'StepStart':
        setSteps((prev) => [
          ...prev,
          {
            step: data.step as string,
            description: data.description as string | undefined,
            status: 'running',
          },
        ]);
        break;

      case 'StepProgress':
        setSteps((prev) =>
          prev.map((s) =>
            s.step === data.step
              ? { ...s, progress: data.progress as number, detail: data.detail as string | undefined }
              : s,
          ),
        );
        break;

      case 'StepComplete':
        setSteps((prev) => prev.map((s) => (s.step === data.step ? { ...s, status: 'complete' as const } : s)));
        break;

      case 'StepError':
        setSteps((prev) =>
          prev.map((s) => (s.step === data.step ? { ...s, status: 'error' as const, error: data.error as string } : s)),
        );
        if (!(data.recoverable as boolean)) {
          setInstallError(data.error as string);
          setInstalling(false);
          eventSourceRef.current?.close();
        }
        break;

      case 'Complete':
        setComplete(true);
        setInstalling(false);
        eventSourceRef.current?.close();
        eventSourceRef.current = null;
        onInstalled();
        break;

      case 'ServerInstall':
        // Informational event, no action needed
        break;
    }
  }

  const allEnvVars = [...entry.env_vars, ...entry.optional_env_vars];

  return (
    <Modal title={t('marketplace.install_title', { name: entry.name })} icon={Package} size="sm" onClose={onClose}>
      <div className="px-5 py-4 space-y-4">
        {/* Server info */}
        <div className="space-y-1">
          <div className="flex items-center gap-2 text-[11px] font-mono text-content-tertiary">
            <span>v{entry.version}</span>
            <span className="px-1.5 rounded bg-surface-secondary uppercase">{entry.category}</span>
            <span className="px-1.5 rounded bg-surface-secondary">{entry.trust_level}</span>
          </div>
          <p className="text-[11px] font-mono text-content-secondary leading-relaxed">{entry.description}</p>
        </div>

        {/* Rust toolchain notice */}
        {entry.runtime === 'rust' && !installing && !complete && (
          <div className="flex items-center gap-2 bg-orange-500/5 border border-orange-500/20 rounded px-3 py-2">
            <AlertTriangle size={12} className="text-orange-400 shrink-0" />
            <span className="text-[11px] font-mono text-orange-400">{t('marketplace.rust_notice')}</span>
          </div>
        )}

        {/* Env vars form */}
        {allEnvVars.length > 0 && !installing && !complete && (
          <div className="space-y-2">
            <span className="text-[11px] font-mono text-content-tertiary uppercase tracking-wider">
              Environment Variables
            </span>
            {allEnvVars.map((ev) => {
              const isRequired = entry.env_vars.some((v) => v.key === ev.key);
              return (
                <div key={ev.key}>
                  <label className="flex items-center gap-1 text-[11px] font-mono text-content-tertiary mb-0.5">
                    {ev.key}
                    {isRequired && <span className="text-red-400">*</span>}
                  </label>
                  {ev.description && (
                    <p className="text-[10px] font-mono text-content-tertiary mb-0.5">{ev.description}</p>
                  )}
                  <input
                    type="text"
                    value={envVars[ev.key] ?? ''}
                    onChange={(e) => handleEnvChange(ev.key, e.target.value)}
                    placeholder={ev.default ?? ''}
                    className="w-full text-[11px] font-mono bg-glass border border-edge rounded px-2 py-1.5 text-content-primary placeholder:text-content-tertiary"
                  />
                </div>
              );
            })}
          </div>
        )}

        {/* Progress steps */}
        {installing && steps.length > 0 && (
          <div className="space-y-2">
            {steps.map((step) => (
              <div key={step.step} className="flex items-center gap-2">
                {step.status === 'running' && <Loader size={12} className="text-brand animate-spin shrink-0" />}
                {step.status === 'complete' && <CheckCircle size={12} className="text-emerald-500 shrink-0" />}
                {step.status === 'error' && <AlertTriangle size={12} className="text-red-500 shrink-0" />}
                {step.status === 'pending' && <span className="w-3 h-3 rounded-full border border-edge shrink-0" />}
                <div className="flex-1 min-w-0">
                  <span className="text-[11px] font-mono text-content-secondary">{step.description ?? step.step}</span>
                  {step.detail && (
                    <span className="text-[10px] font-mono text-content-tertiary ml-2">{step.detail}</span>
                  )}
                  {step.error && <p className="text-[10px] font-mono text-red-400 mt-0.5">{step.error}</p>}
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Complete message */}
        {complete && (
          <div className="flex items-center gap-2 text-emerald-500">
            <CheckCircle size={14} />
            <span className="text-[11px] font-mono">{t('marketplace.complete')}</span>
          </div>
        )}

        {/* Error message */}
        {installError && (
          <div className="flex items-center gap-2 text-red-400 bg-red-500/5 border border-red-500/20 rounded px-3 py-2">
            <AlertTriangle size={12} className="shrink-0" />
            <span className="text-[11px] font-mono">{installError}</span>
          </div>
        )}

        {/* Buttons */}
        {!complete && (
          <div className="flex justify-end gap-2 pt-1">
            <button
              onClick={onClose}
              disabled={installing}
              aria-label={t('marketplace.cancel')}
              className="px-3 py-1.5 text-[11px] font-mono rounded bg-glass hover:bg-glass-strong text-content-tertiary transition-colors border border-edge disabled:opacity-40"
            >
              {t('marketplace.cancel')}
            </button>
            <button
              onClick={handleInstall}
              disabled={installing}
              aria-label={t('marketplace.install_start')}
              className="px-3 py-1.5 text-[11px] font-mono rounded bg-brand/10 hover:bg-brand/20 text-brand disabled:opacity-40 transition-colors border border-brand/20"
            >
              {installing ? t('marketplace.installing') : t('marketplace.install_start')}
            </button>
          </div>
        )}

        {complete && (
          <div className="flex justify-end pt-1">
            <button
              onClick={onClose}
              aria-label="OK"
              className="px-3 py-1.5 text-[11px] font-mono rounded bg-brand/10 hover:bg-brand/20 text-brand transition-colors border border-brand/20"
            >
              OK
            </button>
          </div>
        )}
      </div>
    </Modal>
  );
}
