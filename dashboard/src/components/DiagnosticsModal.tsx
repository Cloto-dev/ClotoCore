import { Check, Copy, Loader2, ShieldAlert, X } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type DiagnosticsMode, fetchDiagnosticsReport, readStoredApiKey } from '../services/api';

interface Props {
  /** The surface that failed, as the UI names it. */
  context?: string;
  /** The message the UI displayed. */
  message?: string;
  /** React component stack, when an ErrorBoundary caught the failure. */
  componentStack?: string;
  onClose: () => void;
}

/**
 * What the browser alone can say, for when the kernel cannot answer.
 *
 * A crash severe enough to reach the ErrorBoundary can also be a kernel that
 * went away, and a report the user cannot produce at all is worse than a thin
 * one. This carries no log and no install receipt — and, because the masking
 * lives kernel-side, it says so rather than implying the text is already safe.
 */
function localFallbackReport(context?: string, message?: string, componentStack?: string): string {
  const description = [context, message].filter(Boolean).join(': ') || '<!-- what happened? -->';
  const stack = componentStack
    ? `\n<details><summary>Component stack</summary>\n\n\`\`\`\n${componentStack.trim()}\n\`\`\`\n\n</details>\n`
    : '';
  return `**Description**
${description}

**Steps to Reproduce**
1.
2.
3.

**Expected Behavior**


**Environment**
- ClotoCore version:
- OS: ${navigator.userAgent}
- Rust version:
${stack}
<!-- The kernel did not answer, so this report carries only what the browser
     knows: no log, no install receipt, and no secret masking. Read it before
     posting. -->
`;
}

/**
 * Shows a pasteable bug report for a failure the user just saw.
 *
 * The text is editable on purpose. The person pasting it is the last check
 * before it reaches a public issue, and that check only works if they can
 * change what they are about to send.
 */
export function DiagnosticsModal({ context, message, componentStack, onClose }: Props) {
  const { t } = useTranslation('common');
  const [mode, setMode] = useState<DiagnosticsMode>('safe');
  const [text, setText] = useState('');
  const [masked, setMasked] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [fromKernel, setFromKernel] = useState(true);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    const apiKey = readStoredApiKey();
    const request = {
      context,
      message,
      component_stack: componentStack,
      mode,
    };
    const run = apiKey ? fetchDiagnosticsReport(apiKey, request) : Promise.reject(new Error('no api key'));

    run
      .then((report) => {
        if (cancelled) return;
        setText(report.markdown);
        setMasked(report.masked);
        setFromKernel(true);
      })
      .catch(() => {
        if (cancelled) return;
        setText(localFallbackReport(context, message, componentStack));
        setMasked(null);
        setFromKernel(false);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [context, message, componentStack, mode]);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  }, [text]);

  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-[var(--surface-overlay)] backdrop-blur-sm p-6">
      <div className="w-full max-w-3xl max-h-full flex flex-col bg-glass backdrop-blur-md border border-edge rounded-lg overflow-hidden">
        <div className="flex items-center justify-between px-4 py-3 border-b border-edge">
          <div className="text-[10px] font-black tracking-[0.2em] text-content-primary uppercase">
            {t('diagnostics_title')}
          </div>
          <button
            type="button"
            onClick={onClose}
            className="text-content-tertiary hover:text-content-primary transition-colors"
            aria-label={t('diagnostics_close')}
          >
            <X size={14} />
          </button>
        </div>

        <div className="px-4 py-3 space-y-3 overflow-y-auto">
          <p className="text-[10px] text-content-tertiary leading-relaxed">{t('diagnostics_description')}</p>

          <div className="flex items-center gap-2">
            {(['safe', 'full'] as const).map((level) => (
              <button
                key={level}
                type="button"
                onClick={() => setMode(level)}
                className={`px-3 py-1 text-[9px] font-bold uppercase tracking-widest rounded border transition-colors ${
                  mode === level
                    ? 'border-brand text-content-primary bg-glass-strong'
                    : 'border-edge text-content-tertiary hover:border-brand'
                }`}
              >
                {t(level === 'safe' ? 'diagnostics_level_safe' : 'diagnostics_level_full')}
              </button>
            ))}
          </div>

          {mode === 'full' && (
            <div className="flex items-start gap-2 text-[10px] text-amber-500">
              <ShieldAlert size={12} className="shrink-0 mt-px" />
              <span>{t('diagnostics_full_warning')}</span>
            </div>
          )}

          {!fromKernel && !loading && (
            <div className="flex items-start gap-2 text-[10px] text-amber-500">
              <ShieldAlert size={12} className="shrink-0 mt-px" />
              <span>{t('diagnostics_kernel_unreachable')}</span>
            </div>
          )}

          {loading ? (
            <div className="flex items-center gap-2 py-8 justify-center text-content-tertiary">
              <Loader2 size={14} className="animate-spin" />
              <span className="text-[10px]">{t('diagnostics_loading')}</span>
            </div>
          ) : (
            <textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              spellCheck={false}
              className="w-full h-72 p-3 text-[10px] font-mono leading-relaxed bg-glass-strong border border-edge rounded text-content-secondary resize-none focus:outline-none focus:border-brand"
            />
          )}

          {/* `n`, not `count`: i18next reads `count` as a plural selector and
              would look for `_one` / `_other` keys this pack does not carry. */}
          {masked !== null && (
            <p className="text-[9px] text-content-tertiary">{t('diagnostics_masked', { n: masked })}</p>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-edge">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-2 text-[9px] font-bold uppercase tracking-widest text-content-tertiary hover:text-content-primary transition-colors"
          >
            {t('diagnostics_close')}
          </button>
          <button
            type="button"
            onClick={handleCopy}
            disabled={loading}
            className="inline-flex items-center gap-2 px-4 py-2 text-[9px] font-bold uppercase tracking-widest text-white bg-brand rounded hover:bg-[#1e3dd6] transition-colors disabled:opacity-50"
          >
            {copied ? <Check size={12} /> : <Copy size={12} />}
            {t(copied ? 'diagnostics_copied' : 'diagnostics_copy')}
          </button>
        </div>
      </div>
    </div>
  );
}
