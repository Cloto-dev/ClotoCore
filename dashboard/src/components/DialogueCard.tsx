import { ArrowRight, ChevronDown, ChevronRight, Loader2 } from 'lucide-react';
import { useState } from 'react';
import type { AgentDialogue } from '../types';

interface DialogueCardProps {
  dialogue: AgentDialogue;
}

function truncate(text: string, lines: number): { truncated: string; isTruncated: boolean } {
  const parts = text.split('\n');
  if (parts.length <= lines) return { truncated: text, isTruncated: false };
  return { truncated: parts.slice(0, lines).join('\n'), isTruncated: true };
}

export function DialogueCard({ dialogue }: DialogueCardProps) {
  const [expanded, setExpanded] = useState(false);

  const isError = dialogue.status === 'error';
  const isPending = dialogue.status === 'pending';
  const indent = dialogue.chain_depth > 1;

  const promptPreview = truncate(dialogue.prompt, expanded ? 999 : 3);
  const responsePreview = dialogue.response ? truncate(dialogue.response, expanded ? 999 : 5) : null;
  const canExpand = promptPreview.isTruncated || (responsePreview?.isTruncated ?? false);

  return (
    <div
      className={`rounded-lg border p-3 transition-colors ${
        isError ? 'border-red-500/40 bg-red-500/5' : 'border-edge bg-glass-subtle'
      } ${indent ? 'ml-4 border-l-2 border-l-brand/30' : ''}`}
    >
      {/* Header: caller → target + engine */}
      <div className="flex items-center justify-between gap-2 mb-2">
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-[10px] font-mono font-bold uppercase tracking-wider text-content-secondary truncate">
            {dialogue.caller_agent_name}
          </span>
          <ArrowRight size={10} className="text-content-tertiary shrink-0" />
          <span className="text-[10px] font-mono font-bold uppercase tracking-wider text-content-secondary truncate">
            {dialogue.target_agent_name}
          </span>
        </div>
        <span className="text-[9px] font-mono text-content-tertiary bg-surface-secondary px-1.5 py-0.5 rounded shrink-0">
          {dialogue.engine_id.replace('mind.', '')}
        </span>
      </div>

      {/* Prompt */}
      <div className="mb-2">
        <div className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary mb-0.5">
          {dialogue.caller_agent_name}
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
          <span className="text-[10px] text-content-tertiary animate-pulse">Awaiting response...</span>
        </div>
      ) : (
        responsePreview && (
          <div>
            <div className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary mb-0.5">
              {isError ? 'Error' : dialogue.target_agent_name}
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
            {new Date(dialogue.timestamp).toLocaleTimeString()}
          </span>
        )}
      </div>
    </div>
  );
}
