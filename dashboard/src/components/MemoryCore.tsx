import { Brain, Download, History, Trash2, Upload, User } from 'lucide-react';
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';
import { useEventStream } from '../hooks/useEventStream';
import { type Metrics, useMetrics } from '../hooks/useMetrics';
import { EVENTS_URL } from '../services/api';
import type { AgentMetadata, Episode, Memory } from '../types';
import { SectionHeader } from './ui/SectionHeader';

const DEBOUNCE_DELAY_MS = 300;

/** Extract a display name from an agent_id like "agent.サフィー___sapphy" */
function agentDisplayName(agentId: string, agentMap: Map<string, string>): string {
  const mapped = agentMap.get(agentId);
  if (mapped) return mapped;
  // Fallback: strip "agent." prefix, take part before "___"
  const stripped = agentId.replace(/^agent\./, '');
  const parts = stripped.split('___');
  return parts[0] || stripped;
}

export const MemoryCore = memo(function MemoryCore({ isWindowMode = false }: { isWindowMode?: boolean }) {
  const { t } = useTranslation('memory');
  const [memories, setMemories] = useState<Memory[]>([]);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null); // null = All
  const api = useApi();
  const { metrics: hookMetrics } = useMetrics();
  const metrics: Metrics = hookMetrics ?? { ram_usage: 'N/A', total_memories: 0, total_requests: 0, total_episodes: 0 };

  // Map agent_id → display name
  const agentMap = useMemo(() => {
    const m = new Map<string, string>();
    for (const a of agents) m.set(a.id, a.name);
    return m;
  }, [agents]);

  // Unique agent IDs that have memories or episodes
  const agentTabs = useMemo(() => {
    const ids = new Set<string>();
    for (const mem of memories) ids.add(mem.agent_id);
    for (const ep of episodes) ids.add(ep.agent_id);
    return Array.from(ids).sort();
  }, [memories, episodes]);

  // Filtered data
  const filteredMemories = useMemo(
    () => (selectedAgent ? memories.filter((m) => m.agent_id === selectedAgent) : memories),
    [memories, selectedAgent],
  );
  const filteredEpisodes = useMemo(
    () => (selectedAgent ? episodes.filter((e) => e.agent_id === selectedAgent) : episodes),
    [episodes, selectedAgent],
  );

  const fetchData = useCallback(async () => {
    try {
      const [memories, episodes, agents] = await Promise.all([api.getMemories(), api.getEpisodes(), api.getAgents()]);
      setMemories(memories);
      setEpisodes(episodes);
      setAgents(agents);
    } catch (error) {
      if (import.meta.env.DEV) console.error('Failed to fetch data', error);
    }
  }, [api.getAgents, api.getEpisodes, api.getMemories]);

  // H-18: Debounce fetchData to prevent cascading API calls on rapid events
  const fetchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const debouncedFetchData = useCallback(() => {
    if (fetchTimeoutRef.current) {
      clearTimeout(fetchTimeoutRef.current);
    }
    fetchTimeoutRef.current = setTimeout(() => {
      fetchData();
    }, DEBOUNCE_DELAY_MS);
  }, [fetchData]);

  useEffect(() => {
    return () => {
      if (fetchTimeoutRef.current) {
        clearTimeout(fetchTimeoutRef.current);
      }
    };
  }, []);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  const handleDeleteMemory = async (id: number) => {
    try {
      await api.deleteMemory(id);
      setMemories((prev) => prev.filter((m) => m.id !== id));
    } catch (e) {
      if (import.meta.env.DEV) console.error('Failed to delete memory:', e);
    }
  };

  const handleDeleteEpisode = async (id: number) => {
    try {
      await api.deleteEpisode(id);
      setEpisodes((prev) => prev.filter((e) => e.id !== id));
    } catch (e) {
      if (import.meta.env.DEV) console.error('Failed to delete episode:', e);
    }
  };

  // --- Export: build JSONL client-side from existing data ---
  const handleExport = useCallback(() => {
    const exportMemories = selectedAgent ? memories.filter((m) => m.agent_id === selectedAgent) : memories;
    const exportEpisodes = selectedAgent ? episodes.filter((e) => e.agent_id === selectedAgent) : episodes;

    const lines: string[] = [];
    // Header
    lines.push(
      JSON.stringify({
        _type: 'header',
        version: 'cpersona-export/1.0',
        agent_id: selectedAgent ?? '',
        exported_at: new Date().toISOString(),
        memory_count: exportMemories.length,
        episode_count: exportEpisodes.length,
        has_profile: false,
      }),
    );
    // Memories
    for (const m of exportMemories) {
      lines.push(
        JSON.stringify({
          _type: 'memory',
          id: m.id,
          agent_id: m.agent_id,
          content: m.content,
          source: m.source,
          timestamp: m.timestamp,
          created_at: m.created_at,
        }),
      );
    }
    // Episodes
    for (const e of exportEpisodes) {
      lines.push(
        JSON.stringify({
          _type: 'episode',
          id: e.id,
          agent_id: e.agent_id,
          summary: e.summary,
          keywords: e.keywords,
          start_time: e.start_time,
          end_time: e.end_time,
          created_at: e.created_at,
        }),
      );
    }

    const blob = new Blob([lines.join('\n')], { type: 'application/x-ndjson' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    const datePart = new Date().toISOString().slice(0, 10);
    const agentPart = selectedAgent ? agentDisplayName(selectedAgent, agentMap).replace(/\s+/g, '_') : 'all';
    a.download = `${agentPart}_memories_${datePart}.jsonl`;
    a.click();
    URL.revokeObjectURL(url);
  }, [memories, episodes, selectedAgent, agentMap]);

  // --- Import: file picker + confirmation + API call ---
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importing, setImporting] = useState(false);

  const handleImportClick = () => fileInputRef.current?.click();

  const handleImportFile = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      // Reset so same file can be re-selected
      e.target.value = '';

      const text = await file.text();
      const lines = text.split('\n').filter((l) => l.trim());

      // Parse header for confirmation
      let memCount = 0;
      let epCount = 0;
      for (const line of lines) {
        try {
          const rec = JSON.parse(line);
          if (rec._type === 'memory') memCount++;
          else if (rec._type === 'episode') epCount++;
        } catch {
          /* skip malformed lines */
        }
      }

      const msg = t('import_confirm', { memories: memCount, episodes: epCount });
      if (!window.confirm(msg)) return;

      setImporting(true);
      try {
        const result = await api.importMemories(text, selectedAgent ?? '');
        const info = t('import_success', {
          memories: result.imported_memories,
          episodes: result.imported_episodes,
          skipped: result.skipped_memories,
        });
        window.alert(info);
        fetchData();
      } catch (err) {
        if (import.meta.env.DEV) console.error('Import failed:', err);
        window.alert(t('import_error'));
      } finally {
        setImporting(false);
      }
    },
    [api, selectedAgent, t, fetchData],
  );

  useEventStream(
    EVENTS_URL,
    (data) => {
      if (
        data.type === '__reconnected' ||
        data.type === '__lagged' ||
        data.type === 'MessageReceived' ||
        data.type === 'VisionUpdated' ||
        data.type === 'SystemNotification'
      ) {
        // H-18: Use debounced fetch to prevent cascading API calls
        debouncedFetchData();
      }
    },
    api.apiKey,
  );

  return (
    <div
      className={`${isWindowMode ? 'bg-transparent p-4' : 'h-full overflow-y-auto'} relative font-sans text-content-primary overflow-x-hidden animate-in fade-in duration-500`}
    >
      {/* Inline header bar with metrics */}
      {!isWindowMode && (
        <div className="px-6 pt-4 pb-2 md:px-12 space-y-2">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Brain className="text-brand" size={16} />
              <h2 className="text-xs font-mono uppercase tracking-widest text-content-primary font-bold">
                {t('title')}
              </h2>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-[10px] font-mono text-content-tertiary">
                {metrics.ram_usage} / {metrics.total_memories} {t('objs')}
              </span>
              <button
                onClick={handleExport}
                disabled={filteredMemories.length === 0 && filteredEpisodes.length === 0}
                title={t('export_tooltip')}
                aria-label={t('export')}
                className="p-1 rounded text-content-tertiary hover:text-brand hover:bg-brand/10 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
              >
                <Upload size={14} />
              </button>
              <button
                onClick={handleImportClick}
                disabled={importing}
                title={t('import_tooltip')}
                aria-label={t('import')}
                className="p-1 rounded text-content-tertiary hover:text-brand hover:bg-brand/10 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
              >
                <Download size={14} />
              </button>
              <input
                ref={fileInputRef}
                type="file"
                accept=".jsonl,.ndjson"
                className="hidden"
                onChange={handleImportFile}
                aria-label={t('import')}
              />
            </div>
          </div>
          {/* Agent filter tabs */}
          {agentTabs.length > 0 && (
            <div className="flex items-center gap-1.5 overflow-x-auto pb-1">
              <button
                onClick={() => setSelectedAgent(null)}
                aria-label={t('all')}
                className={`px-3 py-1 rounded-full text-[10px] font-mono font-bold uppercase tracking-wider transition-colors whitespace-nowrap ${
                  selectedAgent === null
                    ? 'bg-brand text-white'
                    : 'bg-glass-strong text-content-tertiary hover:text-content-secondary border border-edge'
                }`}
              >
                {t('all')}
              </button>
              {agentTabs.map((agentId) => (
                <button
                  key={agentId}
                  onClick={() => setSelectedAgent(agentId)}
                  aria-label={agentDisplayName(agentId, agentMap)}
                  className={`px-3 py-1 rounded-full text-[10px] font-mono font-bold uppercase tracking-wider transition-colors whitespace-nowrap ${
                    selectedAgent === agentId
                      ? 'bg-brand text-white'
                      : 'bg-glass-strong text-content-tertiary hover:text-content-secondary border border-edge'
                  }`}
                >
                  {agentDisplayName(agentId, agentMap)}
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      <div className={`relative z-10 ${isWindowMode ? '' : 'p-6 md:px-12'}`}>
        <main className={`grid grid-cols-1 ${isWindowMode ? 'gap-4' : 'lg:grid-cols-3 gap-8'}`}>
          <section className={`${isWindowMode ? '' : 'lg:col-span-2'} space-y-4`}>
            <SectionHeader icon={User} title={t('long_term')} className="mb-2" />

            <div className={`grid ${isWindowMode ? 'grid-cols-1' : 'grid-cols-1 md:grid-cols-2'} gap-4`}>
              {filteredMemories.length > 0 ? (
                filteredMemories.map((mem) => (
                  <div
                    key={mem.id}
                    className="bg-surface-primary/50 p-4 rounded-xl shadow-sm hover:shadow-md transition-all duration-300 border border-edge hover:border-brand group flex flex-col max-h-48"
                  >
                    <div className="flex items-center gap-3 mb-2">
                      <div className="w-6 h-6 bg-surface-secondary rounded flex items-center justify-center group-hover:bg-brand/10 transition-colors">
                        <User size={12} className="text-content-tertiary group-hover:text-brand" />
                      </div>
                      <span className="text-[10px] font-mono text-content-tertiary">
                        {agentDisplayName(mem.agent_id, agentMap)}
                      </span>
                    </div>
                    <div className="flex-1 min-h-0 text-xs font-medium leading-relaxed text-content-secondary whitespace-pre-wrap line-clamp-6 font-mono">
                      {mem.content}
                    </div>
                    <div className="mt-2 pt-2 border-t border-edge-subtle flex justify-between items-center">
                      <span className="text-[9px] text-content-tertiary font-bold uppercase tracking-widest">
                        {mem.created_at}
                      </span>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          handleDeleteMemory(mem.id);
                        }}
                        className="p-1 rounded text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-all opacity-0 group-hover:opacity-100"
                        title={t('delete_memory')}
                        aria-label={t('delete_memory')}
                      >
                        <Trash2 size={12} />
                      </button>
                    </div>
                  </div>
                ))
              ) : (
                <div className="col-span-full py-8 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
                  {t('no_memories')}
                </div>
              )}
            </div>
          </section>

          <section className="space-y-4">
            <SectionHeader icon={History} title={t('episodic')} className="mb-2" />

            <div className="space-y-3">
              {filteredEpisodes.length > 0 ? (
                filteredEpisodes.map((epi) => (
                  <div
                    key={epi.id}
                    className="bg-surface-primary/50 p-3 rounded-xl border-l-2 border-brand shadow-sm hover:translate-x-1 transition-transform group"
                  >
                    <div className="text-[10px] font-black text-brand mb-1 uppercase tracking-wider flex justify-between items-center">
                      <span>{epi.created_at || 'LOG: RECENT'}</span>
                      <div className="flex items-center gap-1.5">
                        <span className="text-content-tertiary font-mono">
                          {agentDisplayName(epi.agent_id, agentMap)}
                        </span>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            handleDeleteEpisode(epi.id);
                          }}
                          className="p-1 rounded text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-all opacity-0 group-hover:opacity-100"
                          title={t('delete_episode')}
                          aria-label={t('delete_episode')}
                        >
                          <Trash2 size={10} />
                        </button>
                      </div>
                    </div>
                    {epi.keywords && (
                      <div className="text-[9px] font-mono text-content-tertiary mb-1">{epi.keywords}</div>
                    )}
                    <p className="text-xs text-content-secondary line-clamp-3 font-mono leading-relaxed group-hover:text-content-primary">
                      {epi.summary}
                    </p>
                  </div>
                ))
              ) : (
                <div className="py-8 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
                  {t('no_episodes')}
                </div>
              )}
            </div>
          </section>
        </main>
      </div>
    </div>
  );
});
