import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';
import { useEventStream } from '../hooks/useEventStream';
import { buildTimeline, type DayGroup, localDayNumber, matchesSearch, parseMemoryTime } from '../lib/memoryTimeline';
import { EVENTS_URL } from '../services/api';
import type { AgentMetadata, Episode, Memory, MemoryCapabilities } from '../types';

// HARDCODED(crates/core/src/handlers.rs::get_memories): the kernel returns at most this many memories.
const MEMORY_PAGE = 100;
// HARDCODED(crates/core/src/handlers.rs::get_episodes): the kernel returns at most this many episodes.
const EPISODE_PAGE = 50;
const REFRESH_DELAY_MS = 500;
const NO_CAPABILITIES: MemoryCapabilities = {
  update_memory: false,
  lock_memory: false,
  unlock_memory: false,
  set_recall_precision: false,
  get_recall_precision: false,
};

type Target = { kind: 'memory' | 'episode'; id: number };
type MemoryEntry = { key: string; agentId: string; at: Date; memory: Memory };

export function PartnerMemories({
  agent,
  onBack,
  onTalk,
  onManage,
}: {
  agent: AgentMetadata;
  onBack: () => void;
  onTalk: () => void;
  onManage?: () => void;
}) {
  const { t, i18n } = useTranslation('agents');
  const api = useApi();
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [memories, setMemories] = useState<Memory[]>([]);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [capabilities, setCapabilities] = useState<MemoryCapabilities>(NO_CAPABILITIES);
  const [now, setNow] = useState(() => new Date());
  const [query, setQuery] = useState('');
  const [editing, setEditing] = useState<{ id: number; text: string } | null>(null);
  const [confirming, setConfirming] = useState<Target | null>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const loadedOnce = useRef(false);
  const working = useRef(false);
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const keepRef = useRef<HTMLButtonElement>(null);
  const editRef = useRef<HTMLTextAreaElement>(null);
  // Read through a ref so a new translator never re-runs the load (and its kernel calls).
  const tRef = useRef(t);
  tRef.current = t;

  const load = useCallback(async () => {
    try {
      // Scoped to this partner: an unscoped call returns every agent's memories.
      const [memoryResult, episodeResult] = await Promise.all([api.getMemories(agent.id), api.getEpisodes(agent.id)]);
      setMemories(memoryResult.memories);
      setCapabilities(memoryResult.capabilities);
      setEpisodes(episodeResult);
      setNow(new Date());
      loadedOnce.current = true;
      setStatus('ready');
    } catch {
      // A failed refresh keeps what is already shown; only the first load has nothing to fall back on.
      if (loadedOnce.current) setProblem(tRef.current('mate.memories_refresh_failed'));
      else setStatus('error');
    }
  }, [agent.id, api.getEpisodes, api.getMemories]);

  useEffect(() => {
    void load();
  }, [load]);
  const editingId = editing?.id ?? null;
  useEffect(() => {
    if (editingId !== null) editRef.current?.focus();
  }, [editingId]);
  useEffect(() => {
    // Opening a confirmation moves focus to the safe choice; a later refresh does not move it again.
    if (confirming) keepRef.current?.focus({ preventScroll: true });
  }, [confirming]);
  useEffect(
    () => () => {
      if (refreshTimer.current) clearTimeout(refreshTimer.current);
    },
    [],
  );
  useEventStream(
    EVENTS_URL,
    (data) => {
      if (
        data.type === '__reconnected' ||
        data.type === '__lagged' ||
        data.type === 'MessageReceived' ||
        data.type === 'SystemNotification'
      ) {
        if (refreshTimer.current) clearTimeout(refreshTimer.current);
        refreshTimer.current = setTimeout(() => void load(), REFRESH_DELAY_MS);
      }
    },
    api.apiKey,
  );

  async function change(operation: () => Promise<void>) {
    if (working.current) return;
    working.current = true;
    setBusy(true);
    setProblem(null);
    try {
      await operation();
    } catch {
      setProblem(t('mate.memory_change_failed'));
    } finally {
      working.current = false;
      setBusy(false);
    }
  }

  const entries = useMemo<MemoryEntry[]>(
    () =>
      memories.flatMap((memory) => {
        const at = parseMemoryTime(memory.timestamp) ?? parseMemoryTime(memory.created_at);
        return at && matchesSearch([memory.content], query)
          ? [{ key: `m${memory.id}`, agentId: memory.agent_id, at, memory }]
          : [];
      }),
    [memories, query],
  );
  const days = useMemo(
    () => buildTimeline(entries, now).filter((row): row is DayGroup<MemoryEntry> => row.kind === 'day'),
    [entries, now],
  );
  const summaries = useMemo(
    () => episodes.filter((episode) => matchesSearch([episode.summary, episode.keywords], query)),
    [episodes, query],
  );
  const lang = i18n.language;
  const timeFormat = useMemo(() => new Intl.DateTimeFormat(lang, { hour: '2-digit', minute: '2-digit' }), [lang]);
  function dayLabel(date: Date) {
    const distance = localDayNumber(now) - localDayNumber(date);
    if (distance === 0) return t('mate.today');
    if (distance === 1) return t('mate.yesterday');
    return new Intl.DateTimeFormat(lang, {
      month: 'long',
      day: 'numeric',
      weekday: 'short',
      ...(date.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }),
    }).format(date);
  }

  function confirmation(target: Target, message: string, remove: () => Promise<void>) {
    if (confirming?.kind !== target.kind || confirming.id !== target.id) return null;
    // The focused button is read together with the question it answers.
    const question = `mate-confirm-${target.kind}-${target.id}`;
    return (
      <div className="mate-memory-confirm">
        <p id={question}>{message}</p>
        <button
          type="button"
          className="mate-danger"
          disabled={busy}
          aria-describedby={question}
          onClick={() => void change(remove)}
        >
          {t('mate.memory_delete_confirmed')}
        </button>
        <button
          type="button"
          disabled={busy}
          ref={keepRef}
          aria-describedby={question}
          onClick={() => setConfirming(null)}
        >
          {t('mate.memory_keep')}
        </button>
      </div>
    );
  }

  function memoryItem({ memory, at }: MemoryEntry) {
    const time = timeFormat.format(at);
    if (editing?.id === memory.id) {
      const text = editing.text.trim();
      return (
        <li key={memory.id} className="mate-memory">
          <label className="mate-memory-edit">
            {t('mate.memory_edit_label')}
            <textarea
              ref={editRef}
              value={editing.text}
              onChange={(e) => setEditing({ id: memory.id, text: e.target.value })}
            />
          </label>
          <div className="mate-memory-actions">
            <button
              type="button"
              disabled={busy || !text || text === memory.content}
              onClick={() =>
                void change(async () => {
                  await api.updateMemory(memory.id, text);
                  setMemories((all) => all.map((m) => (m.id === memory.id ? { ...m, content: text } : m)));
                  setEditing(null);
                })
              }
            >
              {t('mate.memory_save')}
            </button>
            <button type="button" disabled={busy} onClick={() => setEditing(null)}>
              {t('mate.memory_cancel')}
            </button>
          </div>
        </li>
      );
    }
    return (
      <li key={memory.id} className="mate-memory">
        <p className="mate-memory-text">{memory.content}</p>
        <div className="mate-memory-actions">
          <span>{t(memory.locked ? 'mate.memory_meta_protected' : 'mate.memory_meta', { time })}</span>
          {capabilities.update_memory && !memory.locked && (
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setConfirming(null);
                setEditing({ id: memory.id, text: memory.content });
              }}
            >
              {t('mate.memory_edit')}
            </button>
          )}
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void change(async () => {
                if (memory.locked) await api.unlockMemory(memory.id);
                else await api.lockMemory(memory.id);
                setMemories((all) => all.map((m) => (m.id === memory.id ? { ...m, locked: !memory.locked } : m)));
                setConfirming(null);
              })
            }
          >
            {t(memory.locked ? 'mate.memory_unprotect' : 'mate.memory_protect')}
          </button>
          {!memory.locked && (
            <button
              type="button"
              className="mate-danger"
              disabled={busy}
              onClick={() => {
                setEditing(null);
                setConfirming({ kind: 'memory', id: memory.id });
              }}
            >
              {t('mate.memory_delete')}
            </button>
          )}
        </div>
        {confirmation({ kind: 'memory', id: memory.id }, t('mate.memory_delete_question'), async () => {
          await api.deleteMemory(memory.id);
          setMemories((all) => all.filter((m) => m.id !== memory.id));
          setConfirming(null);
        })}
      </li>
    );
  }

  function episodeItem(episode: Episode) {
    const at = parseMemoryTime(episode.end_time) ?? parseMemoryTime(episode.created_at);
    return (
      <li key={episode.id} className="mate-memory">
        <p className="mate-memory-text">{episode.summary}</p>
        <div className="mate-memory-actions">
          {at && <span>{t('mate.episode_meta', { day: dayLabel(at), time: timeFormat.format(at) })}</span>}
          <button
            type="button"
            className="mate-danger"
            disabled={busy}
            onClick={() => {
              setEditing(null);
              setConfirming({ kind: 'episode', id: episode.id });
            }}
          >
            {t('mate.memory_delete')}
          </button>
        </div>
        {confirmation({ kind: 'episode', id: episode.id }, t('mate.episode_delete_question'), async () => {
          await api.deleteEpisode(episode.id);
          setEpisodes((all) => all.filter((e) => e.id !== episode.id));
          setConfirming(null);
        })}
      </li>
    );
  }

  const nothing = memories.length === 0 && episodes.length === 0;
  const truncated = memories.length >= MEMORY_PAGE || episodes.length >= EPISODE_PAGE;
  return (
    <section className="mate-settings mate-memories" aria-label={t('mate.memories')}>
      <h2>{t('mate.memories')}</h2>
      <p>{t('mate.memories_intro', { name: agent.name })}</p>
      {status === 'loading' && <p role="status">{t('mate.memories_loading')}</p>}
      {status === 'error' && (
        <div className="mate-notice" role="alert">
          <p>{t('mate.memories_failed')}</p>
          <button
            type="button"
            onClick={() => {
              setStatus('loading');
              void load();
            }}
          >
            {t('mate.memories_retry')}
          </button>
        </div>
      )}
      {status === 'ready' && nothing && (
        <div className="mate-notice">
          <p>{t('mate.memories_empty', { name: agent.name })}</p>
          <button type="button" onClick={onTalk}>
            {t('mate.talk')}
          </button>
        </div>
      )}
      {status === 'ready' && !nothing && (
        <>
          <label className="mate-memory-search">
            {t('mate.memories_search')}
            <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} />
          </label>
          {truncated && <p>{t('mate.memories_truncated')}</p>}
          {query.trim() && days.length === 0 && summaries.length === 0 && (
            <p role="status">{t('mate.memories_no_match')}</p>
          )}
          {days.length > 0 && (
            <section className="mate-field" aria-labelledby="mate-remembered">
              <h3 id="mate-remembered">{t('mate.memories_remembered')}</h3>
              {days.map((day) => (
                <div key={day.day}>
                  <h4 className="mate-memory-day">{dayLabel(day.date)}</h4>
                  <ul className="mate-memory-list">{day.events.map(memoryItem)}</ul>
                </div>
              ))}
            </section>
          )}
          {summaries.length > 0 && (
            <section className="mate-field" aria-labelledby="mate-summaries">
              <h3 id="mate-summaries">{t('mate.memories_summaries')}</h3>
              <p>{t('mate.memories_summaries_hint')}</p>
              <ul className="mate-memory-list">{summaries.map(episodeItem)}</ul>
            </section>
          )}
        </>
      )}
      {problem && (
        <p role="alert" className="mate-problem">
          {problem}
        </p>
      )}
      <div className="mate-memory-footer">
        {onManage && (
          <button type="button" onClick={onManage}>
            {t('mate.memories_manage')}
          </button>
        )}
        <button type="button" onClick={onBack}>
          {t('mate.back')}
        </button>
      </div>
    </section>
  );
}
