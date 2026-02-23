import { AccessControlEntry } from '../../types';

interface SummaryItem {
  tool: string;
  allowed: number;
  denied: number;
  inherited: number;
}

interface Props {
  tools: string[];
  entries: AccessControlEntry[];
  serverGrantCount: number;
  onToolClick?: (tool: string) => void;
}

export function McpAccessSummaryBar({ tools, entries, serverGrantCount, onToolClick }: Props) {
  const summary: SummaryItem[] = tools.map(tool => {
    const toolGrants = entries.filter(e => e.entry_type === 'tool_grant' && e.tool_name === tool);
    const allowed = toolGrants.filter(e => e.permission === 'allow').length;
    const denied = toolGrants.filter(e => e.permission === 'deny').length;
    const explicit = allowed + denied;
    const inherited = Math.max(0, serverGrantCount - explicit);
    return { tool, allowed, denied, inherited };
  });

  if (summary.length === 0) {
    return null;
  }

  return (
    <div className="border border-edge rounded bg-glass p-2">
      <div className="text-[9px] font-mono uppercase tracking-widest text-content-muted mb-1.5">Summary</div>
      <div className="space-y-1">
        <div className="grid grid-cols-4 gap-2 text-[9px] font-mono text-content-muted border-b border-edge-subtle pb-1">
          <span>Tool</span>
          <span className="text-center">Allowed</span>
          <span className="text-center">Denied</span>
          <span className="text-center">Inherited</span>
        </div>
        {summary.map(item => (
          <button
            key={item.tool}
            onClick={() => onToolClick?.(item.tool)}
            className="grid grid-cols-4 gap-2 w-full text-left text-[10px] font-mono hover:bg-glass-strong rounded px-0.5 py-0.5 transition-colors"
          >
            <span className="text-content-secondary truncate">{item.tool}</span>
            <span className="text-center text-green-500">{item.allowed > 0 ? `${item.allowed} agent${item.allowed !== 1 ? 's' : ''}` : '—'}</span>
            <span className="text-center text-red-500">{item.denied > 0 ? `${item.denied} agent${item.denied !== 1 ? 's' : ''}` : '—'}</span>
            <span className="text-center text-content-muted">{item.inherited > 0 ? `${item.inherited} agent${item.inherited !== 1 ? 's' : ''}` : '—'}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
