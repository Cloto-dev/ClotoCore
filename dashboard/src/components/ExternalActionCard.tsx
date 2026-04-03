import { ArrowRight, ChevronDown, ChevronRight, Loader2 } from 'lucide-react';
import { useState } from 'react';
import type { ExternalAction } from '../types';

interface ExternalActionCardProps {
  action: ExternalAction;
}

function truncate(text: string, lines: number): { truncated: string; isTruncated: boolean } {
  const parts = text.split('\n');
  if (parts.length <= lines) return { truncated: text, isTruncated: false };
  return { truncated: parts.slice(0, lines).join('\n'), isTruncated: true };
}

/** Source badge color by platform. */
function sourceBadgeClass(source: string): string {
  switch (source) {
    case 'discord':
      return 'bg-[#5865F2]/20 text-[#5865F2]';
    default:
      return 'bg-surface-secondary text-content-tertiary';
  }
}

export function ExternalActionCard({ action }: ExternalActionCardProps) {
  const [expanded, setExpanded] = useState(false);

  const isError = action.status === 'error';
  const isPending = action.status === 'pending';

  const promptPreview = truncate(action.prompt, expanded ? 999 : 3);
  const responsePreview = action.response ? truncate(action.response, expanded ? 999 : 5) : null;
  const canExpand = promptPreview.isTruncated || (responsePreview?.isTruncated ?? false);

  return (
    <div
      className={`rounded-lg border p-3 transition-colors ${
        isError ? 'border-red-500/40 bg-red-500/5' : 'border-edge bg-glass-subtle'
      }`}
    >
      {/* Header: source badge + sender → target + engine */}
      <div className="flex items-center justify-between gap-2 mb-2">
        <div className="flex items-center gap-1.5 min-w-0">
          <span
            className={`text-[9px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded shrink-0 ${sourceBadgeClass(action.source)}`}
          >
            {action.source_label || action.source}
          </span>
          <span className="text-[10px] font-mono font-bold uppercase tracking-wider text-content-secondary truncate">
            {action.sender_name}
          </span>
          <ArrowRight size={10} className="text-content-tertiary shrink-0" />
          <span className="text-[10px] font-mono font-bold uppercase tracking-wider text-content-secondary truncate">
            {action.target_agent_name}
          </span>
        </div>
        <span className="text-[9px] font-mono text-content-tertiary bg-surface-secondary px-1.5 py-0.5 rounded shrink-0">
          {action.engine_id.replace('mind.', '')}
        </span>
      </div>

      {/* Prompt */}
      <div className="mb-2">
        <div className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary mb-0.5">
          {action.sender_name}
        </div>
        <div className="text-[11px] text-content-primary whitespace-pre-wrap break-words leading-relaxed">
          {promptPreview.truncated}
          {!expanded && promptPreview.isTruncated && <span className="text-content-tertiary">...</span>}
        </div>
      </div>

      {/* Response or pending */}
      {isPending ? (
        <div className="flex items-center gap-1.5">
          <Loader2 size={10} className="text-brand animate-spin" />
          <span className="text-[10px] text-content-tertiary animate-pulse">Processing...</span>
        </div>
      ) : (
        responsePreview && (
          <div>
            <div className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary mb-0.5">
              {isError ? 'Error' : action.target_agent_name}
            </div>
            <div
              className={`text-[11px] whitespace-pre-wrap break-words leading-relaxed ${
                isError ? 'text-red-400' : 'text-content-primary'
              }`}
            >
              {responsePreview.truncated}
              {!expanded && responsePreview.isTruncated && <span className="text-content-tertiary">...</span>}
            </div>
          </div>
        )
      )}

      {/* Footer: expand toggle + timestamp */}
      <div className="mt-2 flex items-center justify-between">
        {canExpand && !isPending ? (
          <button
            type="button"
            onClick={() => setExpanded(!expanded)}
            className="flex items-center gap-1 text-[9px] font-bold uppercase tracking-wider text-content-tertiary hover:text-content-secondary transition-colors"
          >
            {expanded ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
            {expanded ? 'Collapse' : 'Expand'}
          </button>
        ) : (
          <span />
        )}
        {!isPending && (
          <span className="text-[10px] font-mono text-content-tertiary">
            {new Date(action.timestamp).toLocaleTimeString()}
          </span>
        )}
      </div>
    </div>
  );
}
