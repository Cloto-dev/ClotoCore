import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { memo } from 'react';
import { Brain, History, User, Trash2 } from 'lucide-react';
import { Memory, Episode, AgentMetadata } from '../types';
import { useEventStream } from '../hooks/useEventStream';
import { useMetrics, Metrics } from '../hooks/useMetrics';
import { useApi } from '../hooks/useApi';
import { EVENTS_URL } from '../services/api';

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
    () => selectedAgent ? memories.filter(m => m.agent_id === selectedAgent) : memories,
    [memories, selectedAgent],
  );
  const filteredEpisodes = useMemo(
    () => selectedAgent ? episodes.filter(e => e.agent_id === selectedAgent) : episodes,
    [episodes, selectedAgent],
  );

  const fetchData = useCallback(async () => {
    try {
      const [memories, episodes, agents] = await Promise.all([
        api.getMemories(),
        api.getEpisodes(),
        api.getAgents(),
      ]);
      setMemories(memories);
      setEpisodes(episodes);
      setAgents(agents);
    } catch (error) {
      console.error('Failed to fetch data', error);
    }
  }, []);

  // H-18: Debounce fetchData to prevent cascading API calls on rapid events
  const fetchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const debouncedFetchData = useCallback(() => {
    if (fetchTimeoutRef.current) {
      clearTimeout(fetchTimeoutRef.current);
    }
    fetchTimeoutRef.current = setTimeout(() => {
      fetchData();
    }, 300);
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
      setMemories(prev => prev.filter(m => m.id !== id));
    } catch (e) {
      console.error('Failed to delete memory:', e);
    }
  };

  const handleDeleteEpisode = async (id: number) => {
    try {
      await api.deleteEpisode(id);
      setEpisodes(prev => prev.filter(e => e.id !== id));
    } catch (e) {
      console.error('Failed to delete episode:', e);
    }
  };

  useEventStream(EVENTS_URL, (data) => {
    if (data.type === 'MessageReceived' || data.type === 'VisionUpdated' || data.type === 'SystemNotification') {
       // H-18: Use debounced fetch to prevent cascading API calls
       debouncedFetchData();
    }
  }, api.apiKey);

  return (
    <div className={`${isWindowMode ? 'bg-transparent p-4' : 'h-full overflow-y-auto'} relative font-sans text-content-primary overflow-x-hidden animate-in fade-in duration-500`}>
      {/* Inline header bar with metrics */}
      {!isWindowMode && (
        <div className="px-6 pt-4 pb-2 md:px-12 space-y-2">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Brain className="text-brand" size={16} />
              <h2 className="text-xs font-mono uppercase tracking-widest text-content-primary font-bold">Memory Core</h2>
            </div>
            <span className="text-[10px] font-mono text-content-tertiary">{metrics.ram_usage} / {metrics.total_memories} OBJS</span>
          </div>
          {/* Agent filter tabs */}
          {agentTabs.length > 0 && (
            <div className="flex items-center gap-1.5 overflow-x-auto pb-1">
              <button
                onClick={() => setSelectedAgent(null)}
                className={`px-3 py-1 rounded-full text-[10px] font-mono font-bold uppercase tracking-wider transition-colors whitespace-nowrap ${
                  selectedAgent === null
                    ? 'bg-brand text-white'
                    : 'bg-glass-strong text-content-tertiary hover:text-content-secondary border border-edge'
                }`}
              >
                All
              </button>
              {agentTabs.map(agentId => (
                <button
                  key={agentId}
                  onClick={() => setSelectedAgent(agentId)}
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
            <div className="flex items-center gap-3 mb-2 border-b border-edge pb-2">
              <User className="text-brand" size={16} />
              <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Long-term Memory Banks</h2>
            </div>
            
            <div className={`grid ${isWindowMode ? 'grid-cols-1' : 'grid-cols-1 md:grid-cols-2'} gap-4`}>
              {filteredMemories.length > 0 ? filteredMemories.map((mem) => (
                <div key={mem.id} className="bg-glass-strong backdrop-blur-sm p-4 rounded-lg shadow-sm hover:shadow-md transition-all duration-300 border border-edge hover:border-brand group">
                  <div className="flex items-center gap-3 mb-2">
                    <div className="w-6 h-6 bg-surface-secondary rounded flex items-center justify-center group-hover:bg-brand/10 transition-colors">
                      <User size={12} className="text-content-tertiary group-hover:text-brand" />
                    </div>
                    <span className="text-[10px] font-mono text-content-tertiary">{agentDisplayName(mem.agent_id, agentMap)}</span>
                  </div>
                  <div className="text-xs font-medium leading-relaxed text-content-secondary whitespace-pre-wrap line-clamp-6 font-mono">
                    {mem.content}
                  </div>
                  <div className="mt-2 pt-2 border-t border-edge-subtle flex justify-between items-center">
                    <span className="text-[9px] text-content-tertiary font-bold uppercase tracking-widest">{mem.created_at}</span>
                    <button
                      onClick={(e) => { e.stopPropagation(); handleDeleteMemory(mem.id); }}
                      className="p-1 rounded text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-all opacity-0 group-hover:opacity-100"
                      title="Delete memory"
                    >
                      <Trash2 size={12} />
                    </button>
                  </div>
                </div>
              )) : (
                 <div className="col-span-full py-8 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
                    No memories archived.
                 </div>
              )}
            </div>
          </section>

          <section className="space-y-4">
            <div className="flex items-center gap-3 mb-2 border-b border-edge pb-2">
              <History className="text-brand" size={16} />
              <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Episodic Stream</h2>
            </div>
            
            <div className="space-y-3">
              {filteredEpisodes.length > 0 ? filteredEpisodes.map((epi) => (
                <div key={epi.id} className="bg-glass-strong backdrop-blur-sm p-3 rounded-lg border-l-2 border-brand shadow-sm hover:translate-x-1 transition-transform group">
                  <div className="text-[10px] font-black text-brand mb-1 uppercase tracking-wider flex justify-between items-center">
                    <span>{epi.created_at || "LOG: RECENT"}</span>
                    <div className="flex items-center gap-1.5">
                      <span className="text-content-muted font-mono">{agentDisplayName(epi.agent_id, agentMap)}</span>
                      <button
                        onClick={(e) => { e.stopPropagation(); handleDeleteEpisode(epi.id); }}
                        className="p-1 rounded text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-all opacity-0 group-hover:opacity-100"
                        title="Delete episode"
                      >
                        <Trash2 size={10} />
                      </button>
                    </div>
                  </div>
                  {epi.keywords && (
                    <div className="text-[9px] font-mono text-content-muted mb-1">{epi.keywords}</div>
                  )}
                  <p className="text-xs text-content-secondary line-clamp-3 font-mono leading-relaxed group-hover:text-content-primary">
                    {epi.summary}
                  </p>
                </div>
              )) : (
                <div className="py-8 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
                  No episodes logged.
                </div>
              )}
            </div>
          </section>
        </main>

      </div>
    </div>
  );
});