import { useTranslation } from 'react-i18next';
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

function AgentCount({ count, t }: { count: number; t: (key: string, opts?: Record<string, unknown>) => string }) {
  if (count === 0) return <>{'\u2014'}</>;
  return <>{t(count === 1 ? 'access.agent_count_one' : 'access.agent_count_other', { count })}</>;
}

export function McpAccessSummaryBar({ tools, entries, serverGrantCount, onToolClick }: Props) {
  const { t } = useTranslation('mcp');

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
      <div className="text-[9px] font-mono uppercase tracking-widest text-content-tertiary mb-1.5">{t('access.summary')}</div>
      <div className="space-y-1">
        <div className="grid grid-cols-4 gap-2 text-[9px] font-mono text-content-tertiary border-b border-edge-subtle pb-1">
          <span>{t('access.tool')}</span>
          <span className="text-center">{t('access.allowed')}</span>
          <span className="text-center">{t('access.denied')}</span>
          <span className="text-center">{t('access.inherited_col')}</span>
        </div>
        {summary.map(item => (
          <button
            key={item.tool}
            onClick={() => onToolClick?.(item.tool)}
            className="grid grid-cols-4 gap-2 w-full text-left text-[10px] font-mono hover:bg-glass-strong rounded px-0.5 py-0.5 transition-colors"
          >
            <span className="text-content-secondary truncate">{item.tool}</span>
            <span className="text-center text-green-500"><AgentCount count={item.allowed} t={t} /></span>
            <span className="text-center text-red-500"><AgentCount count={item.denied} t={t} /></span>
            <span className="text-center text-content-tertiary"><AgentCount count={item.inherited} t={t} /></span>
          </button>
        ))}
      </div>
    </div>
  );
}
