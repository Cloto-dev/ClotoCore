import { useState, useEffect, useRef, useCallback } from 'react';
import { memo } from 'react';
import { Brain, Sparkles, History, Activity, User, ArrowLeft } from 'lucide-react';
import { Link } from 'react-router-dom';
import { SystemHistory } from './SystemHistory';
import { useEventStream } from '../hooks/useEventStream';
import { api, API_BASE } from '../services/api';

interface Memory {
  user_id: string;
  guild_id: string;
  content: string;
  updated_at: string;
}

interface Episode {
  id: number;
  summary: string;
  start_time: string;
  channel_id?: string;
}

interface Metrics {
  ram_usage: string;
  total_memories: number;
}

export const MemoryCore = memo(function MemoryCore({ isWindowMode = false }: { isWindowMode?: boolean }) {
  const [memories, setMemories] = useState<Memory[]>([]);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [metrics, setMetrics] = useState<Metrics>({ ram_usage: 'N/A', total_memories: 0 });

  const fetchData = useCallback(async () => {
    try {
      const [memories, episodes, metrics] = await Promise.all([
        api.getMemories(),
        api.getEpisodes(),
        api.getMetrics()
      ]);
      setMemories(memories);
      setEpisodes(episodes);
      setMetrics(metrics);
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

  useEventStream(`${API_BASE}/events`, (data) => {
    if (data.type === 'MessageReceived' || data.type === 'VisionUpdated' || data.type === 'SystemNotification') {
       // H-18: Use debounced fetch to prevent cascading API calls
       debouncedFetchData();
    }
  });

  return (
    <div className={`${isWindowMode ? 'bg-transparent p-4' : 'bg-surface-base min-h-screen'} relative font-sans text-content-primary overflow-x-hidden h-full animate-in fade-in duration-500`}>
      {!isWindowMode && (
        <div 
          className="fixed inset-0 z-0 opacity-30 pointer-events-none"
          style={{
            backgroundImage: `linear-gradient(to right, #cbd5e1 1px, transparent 1px), linear-gradient(to bottom, #cbd5e1 1px, transparent 1px)`,
            backgroundSize: '40px 40px',
            maskImage: 'linear-gradient(to bottom, black 40%, transparent 100%)',
            WebkitMaskImage: 'linear-gradient(to bottom, black 40%, transparent 100%)'
          }}
        />
      )}

      <div className={`relative z-10 ${isWindowMode ? '' : 'p-6 md:p-12'}`}>
        {!isWindowMode && (
          <header className="flex flex-col md:flex-row md:items-center justify-between gap-6 mb-12">
            <div className="flex items-center gap-6">
              <Link to="/" className="p-3 rounded-full bg-glass-subtle backdrop-blur-sm border border-edge hover:border-brand hover:text-brand transition-all shadow-sm group">
                <ArrowLeft size={20} className="group-hover:-translate-x-1 transition-transform" />
              </Link>
              <div className="flex items-center gap-4">
                <div className="w-12 h-12 bg-glass-subtle backdrop-blur-sm rounded-md flex items-center justify-center shadow-sm border border-edge">
                  <Brain className="text-brand" size={24} strokeWidth={2} />
                </div>
                <div>
                  <h1 className="text-3xl font-black tracking-tighter text-content-primary uppercase">Memory Core</h1>
                  <p className="text-[10px] text-content-tertiary font-mono uppercase tracking-[0.2em] flex items-center gap-2">
                    <span className="inline-block w-1.5 h-1.5 bg-brand rounded-full animate-pulse"></span>
                    KS2.1 Storage Interface
                  </p>
                </div>
              </div>
            </div>
            
            <div className="bg-glass-subtle backdrop-blur-sm px-6 py-3 rounded-md flex items-center gap-6 shadow-sm border border-edge">
              <div className="flex flex-col items-end">
                <span className="text-[9px] uppercase font-bold text-content-tertiary tracking-widest">System Load</span>
                <span className="text-sm font-mono font-bold text-content-primary">{metrics.ram_usage} / {metrics.total_memories} OBJS</span>
              </div>
              <Activity className="text-brand" size={20} />
            </div>
          </header>
        )}

        <main className={`grid grid-cols-1 ${isWindowMode ? 'gap-4' : 'lg:grid-cols-3 gap-8'}`}>
          <section className={`${isWindowMode ? '' : 'lg:col-span-2'} space-y-4`}>
            <div className="flex items-center gap-3 mb-2 border-b border-edge pb-2">
              <User className="text-brand" size={16} />
              <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Long-term Memory Banks</h2>
            </div>
            
            <div className={`grid ${isWindowMode ? 'grid-cols-1' : 'grid-cols-1 md:grid-cols-2'} gap-4`}>
              {memories.length > 0 ? memories.map((mem) => (
                <div key={`${mem.user_id}-${mem.guild_id}`} className="bg-glass-strong backdrop-blur-sm p-4 rounded-lg shadow-sm hover:shadow-md transition-all duration-300 border border-edge hover:border-brand group">
                  <div className="flex items-center gap-3 mb-2">
                    <div className="w-6 h-6 bg-surface-secondary rounded flex items-center justify-center group-hover:bg-brand/10 transition-colors">
                      <User size={12} className="text-content-tertiary group-hover:text-brand" />
                    </div>
                    <span className="text-[10px] font-mono text-content-tertiary">UID: {mem.user_id.slice(-6)}</span>
                  </div>
                  <div className="text-xs font-medium leading-relaxed text-content-secondary whitespace-pre-wrap line-clamp-6 font-mono">
                    {mem.content}
                  </div>
                  <div className="mt-2 pt-2 border-t border-edge-subtle flex justify-between items-center">
                    <span className="text-[9px] text-content-tertiary font-bold uppercase tracking-widest">{mem.updated_at}</span>
                    <Sparkles size={12} className="text-content-muted group-hover:text-brand" />
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
              {episodes.length > 0 ? episodes.map((epi) => (
                <div key={epi.id} className="bg-glass-strong backdrop-blur-sm p-3 rounded-lg border-l-2 border-brand shadow-sm hover:translate-x-1 transition-transform group">
                  <div className="text-[10px] font-black text-brand mb-1 uppercase tracking-wider flex justify-between">
                    <span>{epi.start_time || "LOG: RECENT"}</span>
                    {epi.channel_id && <span className="text-content-muted font-mono">#{epi.channel_id}</span>}
                  </div>
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

        {!isWindowMode && <SystemHistory />}
      </div>
    </div>
  );
});