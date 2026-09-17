import { Search } from 'lucide-react';
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import { useApi } from '../hooks/useApi';
import { useEventStream } from '../hooks/useEventStream';
import { type Metrics, useMetrics } from '../hooks/useMetrics';
import { agentColor } from '../lib/agentIdentity';
import {
  buildDensity,
  buildTimeline,
  densityWidth,
  matchesSearch,
  parseMemoryTime,
  type TimelineEvent,
} from '../lib/memoryTimeline';
import { EVENTS_URL } from '../services/api';
import type { AgentMetadata, Episode, Memory, MemoryCapabilities } from '../types';
import './Memory.css';
import './Workshop.css';

const DEBOUNCE_DELAY_MS = 300;
/** Days in the band on the right. */
const DENSITY_DAYS = 30;

/** Extract a display name from an agent_id like "agent.サフィー___sapphy" */
function agentDisplayName(agentId: string, agentMap: Map<string, string>): string {
  const mapped = agentMap.get(agentId);
  if (mapped) return mapped;
  // Fallback: strip "agent." prefix, take part before "___"
  const stripped = agentId.replace(/^agent\./, '');
  const parts = stripped.split('___');
  return parts[0] || stripped;
}

/** Extract the speaker name from a memory's source field.
 *  Handles both internally-tagged {"type":"User","name":"...","id":"..."} and
 *  legacy externally-tagged {"User":{"name":"..."}} formats. */
function memorySpeakerName(source: Record<string, unknown>, agentId: string, agentMap: Map<string, string>): string {
  if (!source || typeof source !== 'object') return agentDisplayName(agentId, agentMap);
  // Internally-tagged: { type: "User", name: "sample-user", id: "discord:123" }
  if (source.type === 'User') {
    const name = source.name as string | undefined;
    if (name && name !== 'User') return name;
    // Fallback: extract name from id (e.g. "discord:username" → "username")
    const id = source.id as string | undefined;
    if (id?.includes(':')) return id.split(':').slice(1).join(':');
    return 'User';
  }
  if (source.type === 'Agent') return agentDisplayName(agentId, agentMap);
  // Externally-tagged: { User: { name: "sample-user" } }
  const userObj = source.User ?? source.user;
  if (userObj && typeof userObj === 'object') {
    const name = (userObj as Record<string, string>).name;
    if (name && name !== 'User') return name;
    return 'User';
  }
  if (source.Agent || source.agent) return agentDisplayName(agentId, agentMap);
  // System/profile sources
  if (source.type === 'System' || source.System) return 'System';
  // Default to agent name
  return agentDisplayName(agentId, agentMap);
}

/** Which kind of memory the axis is showing. */
type Kind = 'all' | 'long' | 'episodes';

/** A row on the axis: a long-term memory, or an episode. */
type AxisEvent = TimelineEvent & { who: string; text: string } & (
    | { type: 'memory'; memory: Memory }
    | { type: 'episode'; episode: Episode }
  );

/**
 * The memory screen (docs/gui/samples/06-memory.html): one vertical time axis,
 * newest first. A day is a band, a memory is a point beside what was
 * remembered, and a run of days with nothing on them is compressed into one
 * dotted segment that says how long it was.
 */
export const MemoryCore = memo(function MemoryCore() {
  const { t, i18n } = useTranslation('memory');
  const [memories, setMemories] = useState<Memory[]>([]);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null); // null = All
  const [kind, setKind] = useState<Kind>('all');
  const [query, setQuery] = useState('');
  // ?q= narrows the axis to what search was asked for, then leaves the URL.
  const [searchParams, setSearchParams] = useSearchParams();
  const queryParam = searchParams.get('q');
  useEffect(() => {
    if (queryParam === null) return;
    setQuery(queryParam);
    const next = new URLSearchParams(searchParams);
    next.delete('q');
    setSearchParams(next, { replace: true });
  }, [queryParam, searchParams, setSearchParams]);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [now, setNow] = useState(() => new Date());
  const [capabilities, setCapabilities] = useState<MemoryCapabilities>({
    update_memory: false,
    lock_memory: false,
    unlock_memory: false,
    set_recall_precision: false,
    get_recall_precision: false,
  });
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editContent, setEditContent] = useState('');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const api = useApi();
  const { metrics: hookMetrics } = useMetrics();
  const metrics: Metrics = hookMetrics ?? { ram_usage: 'N/A', total_memories: 0, total_requests: 0, total_episodes: 0 };

  // Dates are written in the reader's language, not in ours.
  const lang = i18n?.language || 'en';
  const dayFormat = useMemo(() => new Intl.DateTimeFormat(lang, { month: 'long', day: 'numeric' }), [lang]);
  const dayYearFormat = useMemo(
    () => new Intl.DateTimeFormat(lang, { year: 'numeric', month: 'long', day: 'numeric' }),
    [lang],
  );
  const timeFormat = useMemo(
    () => new Intl.DateTimeFormat(lang, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }),
    [lang],
  );
  const cellFormat = useMemo(() => new Intl.DateTimeFormat(lang, { month: 'numeric', day: 'numeric' }), [lang]);

  // Map agent_id → display name
  const agentMap = useMemo(() => {
    const m = new Map<string, string>();
    for (const a of agents) m.set(a.id, a.name);
    return m;
  }, [agents]);

  const tabColor = useCallback(
    (agentId: string) => agentColor(agents.find((a) => a.id === agentId) ?? { id: agentId }),
    [agents],
  );

  // Filter tabs: every configured agent, plus any agent_id present in the current
  // data (covers legacy/orphaned memories whose agent is no longer configured).
  // Derived from the global agent list (not just the fetched memory set) so the
  // tab list stays stable when the fetch is scoped to a single selected agent.
  const agentTabs = useMemo(() => {
    const ids = new Set<string>();
    for (const a of agents) ids.add(a.id);
    for (const mem of memories) ids.add(mem.agent_id);
    for (const ep of episodes) ids.add(ep.agent_id);
    // bug-416: never offer an empty-string ("") tab. The backend cannot
    // distinguish agent_id="" from "all agents" — both map to the global
    // most-recent view (see get_memories) — so a "" tab could not be correctly
    // scoped and selecting it would re-trigger bug-413 for global-pool rows
    // outside the top-N window. Those rows are shown under "All" instead.
    ids.delete('');
    return Array.from(ids).sort();
  }, [agents, memories, episodes]);

  // The agent tab and the search narrow together: a row has to pass both.
  const filteredMemories = useMemo(
    () =>
      memories.filter((m) => {
        if (selectedAgent && m.agent_id !== selectedAgent) return false;
        const at = parseMemoryTime(m.timestamp) ?? parseMemoryTime(m.created_at);
        return matchesSearch(
          [
            m.content,
            memorySpeakerName(m.source as Record<string, unknown>, m.agent_id, agentMap),
            agentDisplayName(m.agent_id, agentMap),
            m.created_at,
            at ? dayYearFormat.format(at) : undefined,
          ],
          query,
        );
      }),
    [memories, selectedAgent, query, agentMap, dayYearFormat],
  );
  const filteredEpisodes = useMemo(
    () =>
      episodes.filter((e) => {
        if (selectedAgent && e.agent_id !== selectedAgent) return false;
        const at = parseMemoryTime(e.end_time) ?? parseMemoryTime(e.created_at);
        return matchesSearch(
          [
            e.summary,
            e.keywords,
            agentDisplayName(e.agent_id, agentMap),
            e.created_at,
            at ? dayYearFormat.format(at) : undefined,
          ],
          query,
        );
      }),
    [episodes, selectedAgent, query, agentMap, dayYearFormat],
  );

  // What goes on the axis. "All" and "long-term only" put the memories there;
  // "episodes only" puts the episodes there, so the two tabs never draw the
  // same row twice (the panel on the right holds the episodes otherwise).
  const axisEvents = useMemo<AxisEvent[]>(() => {
    if (kind === 'episodes') {
      const rows: AxisEvent[] = [];
      for (const e of filteredEpisodes) {
        const at = parseMemoryTime(e.end_time) ?? parseMemoryTime(e.created_at);
        if (!at) continue;
        rows.push({
          key: `e${e.id}`,
          agentId: e.agent_id,
          at,
          who: agentDisplayName(e.agent_id, agentMap),
          text: e.summary,
          type: 'episode',
          episode: e,
        });
      }
      return rows;
    }
    const rows: AxisEvent[] = [];
    for (const m of filteredMemories) {
      const at = parseMemoryTime(m.timestamp) ?? parseMemoryTime(m.created_at);
      if (!at) continue;
      rows.push({
        key: `m${m.id}`,
        agentId: m.agent_id,
        at,
        who: memorySpeakerName(m.source as Record<string, unknown>, m.agent_id, agentMap),
        text: m.content,
        type: 'memory',
        memory: m,
      });
    }
    return rows;
  }, [kind, filteredMemories, filteredEpisodes, agentMap]);

  const rows = useMemo(() => buildTimeline(axisEvents, now), [axisEvents, now]);
  const density = useMemo(() => buildDensity(axisEvents, now, DENSITY_DAYS), [axisEvents, now]);
  const busiestDay = useMemo(() => density.reduce((most, c) => Math.max(most, c.count), 0), [density]);

  const loadedOnce = useRef(false);
  // Read through a ref: `t` is a new function whenever the language changes,
  // and a fetch that depended on it would refetch for that alone.
  const tRef = useRef(t);
  tRef.current = t;
  const fetchData = useCallback(async () => {
    try {
      // Scope memory/episode fetch to the selected agent (null = global "All" view)
      // so an agent whose memories aren't among the global most-recent set is still
      // shown in full. The agent list is fetched globally to keep every filter tab
      // visible regardless of the active scope.
      // bug-416: treat both null and "" as the unscoped "All" view — the backend
      // maps agent_id="" to the global view anyway, so only a concrete (non-empty)
      // agent id scopes the fetch.
      const scope = selectedAgent || undefined;
      const [memResult, episodes, agents] = await Promise.all([
        api.getMemories(scope),
        api.getEpisodes(scope),
        api.getAgents(),
      ]);
      setMemories(memResult.memories);
      setCapabilities(memResult.capabilities);
      setEpisodes(episodes);
      setAgents(agents);
      setNow(new Date());
      loadedOnce.current = true;
      setStatus('ready');
    } catch (error) {
      // Only the first load has nothing to fall back on. A refresh that fails —
      // they are fired by every kernel event — keeps what is already on the
      // screen and says so, rather than replacing a full axis with an error.
      if (loadedOnce.current) setErrorMsg(tRef.current('operation_failed'));
      else setStatus('error');
      if (import.meta.env.DEV) console.error('Failed to fetch data', error);
    }
  }, [api.getAgents, api.getEpisodes, api.getMemories, selectedAgent]);

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

  // Auto-clear error toast after 3 seconds
  useEffect(() => {
    if (errorMsg) {
      const timer = setTimeout(() => setErrorMsg(null), 3000);
      return () => clearTimeout(timer);
    }
  }, [errorMsg]);

  const handleDeleteMemory = async (id: number) => {
    try {
      await api.deleteMemory(id);
      setMemories((prev) => prev.filter((m) => m.id !== id));
    } catch (e) {
      setErrorMsg(t('operation_failed'));
      if (import.meta.env.DEV) console.error('Failed to delete memory:', e);
    }
  };

  const handleDeleteEpisode = async (id: number) => {
    try {
      await api.deleteEpisode(id);
      setEpisodes((prev) => prev.filter((e) => e.id !== id));
    } catch (e) {
      setErrorMsg(t('operation_failed'));
      if (import.meta.env.DEV) console.error('Failed to delete episode:', e);
    }
  };

  const handleStartEdit = (mem: Memory) => {
    setEditingId(mem.id);
    setEditContent(mem.content);
  };

  const handleCancelEdit = () => {
    setEditingId(null);
    setEditContent('');
  };

  const handleSaveEdit = async (id: number) => {
    try {
      await api.updateMemory(id, editContent);
      setMemories((prev) => prev.map((m) => (m.id === id ? { ...m, content: editContent } : m)));
      setEditingId(null);
      setEditContent('');
    } catch (e) {
      setErrorMsg(t('operation_failed'));
      if (import.meta.env.DEV) console.error('Failed to update memory:', e);
    }
  };

  const handleToggleLock = async (id: number, currentlyLocked: boolean) => {
    try {
      if (currentlyLocked) {
        const result = await api.unlockMemory(id);
        setMemories((prev) => prev.map((m) => (m.id === id ? { ...m, locked: false, lock_level: undefined } : m)));
        if (import.meta.env.DEV) console.log('Unlocked memory:', id, result);
      } else {
        const result = await api.lockMemory(id);
        const lockLevel = (result.lock_level as 'server' | 'kernel') || 'kernel';
        setMemories((prev) => prev.map((m) => (m.id === id ? { ...m, locked: true, lock_level: lockLevel } : m)));
      }
    } catch (e) {
      setErrorMsg(t('operation_failed'));
      if (import.meta.env.DEV) console.error('Failed to toggle lock:', e);
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

  const nothingAtAll = memories.length === 0 && episodes.length === 0;
  // An agent tab narrows too (it scopes the fetch), so an empty screen under
  // one of them is "nothing matches", not "nothing has been remembered".
  const narrowing = selectedAgent !== null || query.trim() !== '' || kind !== 'all';
  // The kernel answers "Unknown" when it cannot measure the memory in use;
  // saying so in the head would be a word about nothing.
  const ram = metrics.ram_usage;
  const ramKnown = !!ram && ram !== 'N/A' && ram !== 'Unknown';

  function dayLabel(group: { date: Date; isToday: boolean }): string {
    if (group.isToday) return t('today');
    const sameYear = group.date.getFullYear() === now.getFullYear();
    return (sameYear ? dayFormat : dayYearFormat).format(group.date);
  }

  function axis() {
    if (status === 'loading') return <div className="mem-state">{t('loading')}</div>;
    if (status === 'error') {
      return (
        <div className="mem-state">
          <div>{t('operation_failed')}</div>
          <button type="button" className="btn" onClick={() => fetchData()}>
            {t('retry')}
          </button>
        </div>
      );
    }
    if (rows.length === 0) {
      return <div className="mem-state">{nothingAtAll && !narrowing ? t('nothing_yet') : t('no_match')}</div>;
    }
    return (
      <div className="tl">
        {rows.map((row, i) =>
          row.kind === 'gap' ? (
            <div className="gap" key={`gap-${i}`}>
              <span className="sp" />
              <span className="n">{t('gap_days', { count: row.days })}</span>
            </div>
          ) : (
            <div key={row.day}>
              <div className="tday">
                <span className="lbl">{dayLabel(row)}</span>
                <span className="tick" />
                <span className="n">{t('day_count', { count: row.events.length })}</span>
              </div>
              {row.events.map((ev) => (
                <div className="ev" key={ev.key}>
                  <span className="t num">{timeFormat.format(ev.at)}</span>
                  <span className="sp" />
                  {/* The row takes focus so the keyboard can open it and reach
                      what it can be told to do. */}
                  {/* biome-ignore lint/a11y/noNoninteractiveTabindex: the row is what expands; focus is the keyboard's way to do what hover does */}
                  <div className="c" tabIndex={0} data-testid={`memory-row-${ev.key}`}>
                    <div className="who">{ev.who}</div>
                    {ev.type === 'memory' && editingId === ev.memory.id ? (
                      <div className="edit">
                        <textarea
                          className="in"
                          value={editContent}
                          onChange={(e) => setEditContent(e.target.value)}
                          aria-label={t('edit_memory')}
                        />
                        <div className="acts">
                          <button type="button" onClick={() => handleSaveEdit(ev.memory.id)}>
                            {t('save')}
                          </button>
                          <button type="button" onClick={handleCancelEdit}>
                            {t('cancel')}
                          </button>
                        </div>
                      </div>
                    ) : (
                      <div className="tx">{ev.text}</div>
                    )}
                    {ev.type === 'memory' && editingId !== ev.memory.id && (
                      <div className="acts">
                        {capabilities.update_memory && !ev.memory.locked && (
                          <button type="button" onClick={() => handleStartEdit(ev.memory)}>
                            {t('edit_memory')}
                          </button>
                        )}
                        <button type="button" onClick={() => handleToggleLock(ev.memory.id, !!ev.memory.locked)}>
                          {ev.memory.locked ? t('unlock_memory') : t('lock_memory')}
                        </button>
                        <button
                          type="button"
                          className="danger"
                          disabled={!!ev.memory.locked}
                          title={ev.memory.locked ? t('memory_locked') : undefined}
                          onClick={() => handleDeleteMemory(ev.memory.id)}
                        >
                          {t('delete_memory')}
                        </button>
                      </div>
                    )}
                    {ev.type === 'episode' && (
                      <div className="acts">
                        <button type="button" className="danger" onClick={() => handleDeleteEpisode(ev.episode.id)}>
                          {t('delete_episode')}
                        </button>
                      </div>
                    )}
                  </div>
                </div>
              ))}
            </div>
          ),
        )}
      </div>
    );
  }

  return (
    <div className="ws mem" data-testid="memory-page">
      <div className="ws-head">
        <h1>{t('title')}</h1>
        <span className="count">
          {t('counts', { memories: filteredMemories.length, episodes: filteredEpisodes.length })}
        </span>
        {ramKnown && <span className="count">{t('ram_in_use', { ram })}</span>}
        <span className="spacer" />
        <div className="find wide">
          <Search aria-hidden="true" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t('search_placeholder')}
            aria-label={t('search_placeholder')}
          />
        </div>
        <button type="button" className="btn" onClick={() => fetchData()}>
          {t('refresh')}
        </button>
        <button
          type="button"
          className="btn"
          onClick={handleExport}
          disabled={filteredMemories.length === 0 && filteredEpisodes.length === 0}
          title={t('export_tooltip')}
        >
          {t('export')}
        </button>
        <button
          type="button"
          className="btn"
          onClick={handleImportClick}
          disabled={importing}
          title={t('import_tooltip')}
        >
          {importing ? t('importing') : t('import')}
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

      <div className="tabs">
        <button type="button" className={selectedAgent === null ? 'on' : ''} onClick={() => setSelectedAgent(null)}>
          {t('all')}
        </button>
        {agentTabs.map((agentId) => (
          <button
            type="button"
            key={agentId}
            className={selectedAgent === agentId ? 'on' : ''}
            // The one place this screen wears an agent's colour: the line under
            // the name of the agent whose memories are being read.
            style={selectedAgent === agentId ? { borderBottomColor: tabColor(agentId) } : undefined}
            onClick={() => setSelectedAgent(agentId)}
          >
            {agentDisplayName(agentId, agentMap)}
          </button>
        ))}
        <span className="spacer" />
        <button
          type="button"
          className={kind === 'long' ? 'on' : ''}
          onClick={() => setKind(kind === 'long' ? 'all' : 'long')}
        >
          {t('long_term_only')}
        </button>
        <button
          type="button"
          className={kind === 'episodes' ? 'on' : ''}
          onClick={() => setKind(kind === 'episodes' ? 'all' : 'episodes')}
        >
          {t('episodes_only')}
        </button>
      </div>

      {errorMsg && <div className="mem-toast">{errorMsg}</div>}

      <div className="ws-body mem-body">
        <div className="mem-axis">{axis()}</div>
        <aside className="mem-side">
          {kind !== 'episodes' && (
            <>
              <h2>{t('episodic')}</h2>
              {filteredEpisodes.length > 0 ? (
                filteredEpisodes.map((epi) => {
                  const at = parseMemoryTime(epi.end_time) ?? parseMemoryTime(epi.created_at);
                  return (
                    <div className="mem-ep" key={epi.id}>
                      <div className="when">
                        {t('episode_meta', {
                          agent: agentDisplayName(epi.agent_id, agentMap),
                          when: at ? dayYearFormat.format(at) : epi.created_at,
                        })}
                      </div>
                      <p className="sum">{epi.summary}</p>
                      {epi.keywords && <div className="kw">{epi.keywords}</div>}
                      <div className="acts">
                        <button type="button" onClick={() => handleDeleteEpisode(epi.id)}>
                          {t('delete_episode')}
                        </button>
                      </div>
                    </div>
                  );
                })
              ) : (
                <>
                  <p className="said">{t('no_episodes')}</p>
                  <p className="next">{t('episodes_how')}</p>
                </>
              )}
            </>
          )}
          <h2 className={kind === 'episodes' ? '' : 'later'}>{t('last_30_days')}</h2>
          <div className="note">{t('density_note_plain')}</div>
          <div className="dens" data-testid="memory-density">
            {density
              .slice()
              .reverse()
              .map((cell) => (
                <div key={cell.date.getTime()} className={cell.isToday ? 'd today' : 'd'}>
                  <span>{cellFormat.format(cell.date)}</span>
                  <i
                    style={{ width: densityWidth(cell.count, busiestDay) }}
                    title={t('day_count', { count: cell.count })}
                  />
                </div>
              ))}
          </div>
        </aside>
      </div>
    </div>
  );
});
