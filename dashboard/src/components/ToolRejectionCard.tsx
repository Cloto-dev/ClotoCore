import { AlertOctagon, X } from 'lucide-react';
import type { PendingRejection } from '../types';

interface Props {
  rejection: PendingRejection;
  onDismiss: (localId: string) => void;
}

/**
 * Displays a kernel-issued tool rejection as a dismissable notification card.
 * Informational only — no action buttons (security decision: rejection must
 * not carry an "Enable YOLO" shortcut, see plan §5 / woolly-herding-eagle.md).
 * The operator flips settings manually via Settings → Security.
 *
 * Styling policy: functional UI surface → `bg-surface-field` with a red
 * accent for the alert indicator. Hover on the dismiss button uses
 * `hover:border-agent` since dismissing is non-destructive.
 */
export function ToolRejectionCard({ rejection, onDismiss }: Props) {
  return (
    <div className="flex items-start gap-3">
      <div className="w-8 h-8 rounded-lg flex items-center justify-center shrink-0 shadow-sm bg-red-500/20 text-red-400">
        <AlertOctagon size={14} />
      </div>
      <div className="p-4 rounded-xl border border-edge bg-surface-field text-sm max-w-[80%] space-y-2 animate-in fade-in slide-in-from-bottom-2 duration-300 relative">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <span className="text-xs font-bold text-red-400">Tool Rejected</span>
            <code className="text-xs font-mono text-content-tertiary">{rejection.code}</code>
          </div>
          <button
            type="button"
            onClick={() => onDismiss(rejection.local_id)}
            aria-label="Dismiss rejection"
            className="p-1 rounded-md text-content-tertiary hover:text-content-primary hover:bg-surface-panel transition-colors"
          >
            <X size={12} />
          </button>
        </div>
        <div className="text-xs text-content-secondary">
          <span className="font-mono text-content-tertiary">tool: </span>
          <span className="font-mono">{rejection.tool_name}</span>
        </div>
        <p className="text-xs text-content-secondary leading-relaxed whitespace-pre-wrap">{rejection.reason}</p>
        {rejection.remediation_hint && (
          <p className="text-xs text-content-tertiary border-t border-edge pt-2">
            <span className="font-bold">Remediation: </span>
            {rejection.remediation_hint}
          </p>
        )}
        {!rejection.retryable && (
          <p className="text-xs text-red-400/70 italic">Hard rejection — operator action cannot resolve this.</p>
        )}
      </div>
    </div>
  );
}
