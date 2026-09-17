import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useAgentContext } from '../contexts/AgentContext';
import { useConversations } from '../contexts/ConversationContext';
import { useApi } from '../hooks/useApi';
import { useMcpServers } from '../hooks/useMcpServers';
import { displayTitle } from '../lib/conversations';
import { displayServerId } from '../lib/format';
import type { ChatSearchHit, Memory } from '../types';
import './CommandPalette.css';

type Group = 'screens' | 'conversations' | 'messages' | 'servers' | 'memories';

interface Item {
  key: string;
  group: Group;
  label: string;
  detail?: string;
  run: () => void;
}

/** How many rows a group shows; the rest are narrowed to by typing more. */
const PER_GROUP = 5;

/** How long typing has to pause before what was said is searched. */
const SEARCH_PAUSE_MS = 200;

const SETTINGS_SECTIONS = ['general', 'conversations', 'security', 'advanced', 'health', 'log', 'about'] as const;

/** Case- and width-insensitive enough for a name typed from memory. */
function includes(haystack: string | undefined, needle: string): boolean {
  return (haystack ?? '').toLowerCase().includes(needle);
}

/**
 * ⌘K: go anywhere by name — a screen, a conversation, something said in one,
 * an MCP server, or a memory.
 *
 * Conversations are matched here by their titles and the agent's name. What
 * was said inside them is searched by the kernel (`GET /api/chat/search`),
 * which counts every match: the group shows the first few and says how many
 * more there are, rather than a page that reads as the whole answer. Memories
 * are the recent set the kernel lists, and the group says so.
 */
export function CommandPalette({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation('nav');
  const { t: ts } = useTranslation('settings');
  const { t: ta } = useTranslation('agents');
  const navigate = useNavigate();
  const api = useApi();
  const { agents, setSelectedAgentId, setSystemActive } = useAgentContext();
  const { conversations, open, leaveDraft } = useConversations();
  const { servers } = useMcpServers();
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const [memories, setMemories] = useState<{ status: 'loading' | 'ready' | 'error'; items: Memory[] }>({
    status: 'loading',
    items: [],
  });
  const [said, setSaid] = useState<{
    status: 'idle' | 'loading' | 'ready' | 'error';
    hits: ChatSearchHit[];
    total: number;
  }>({ status: 'idle', hits: [], total: 0 });
  const listId = useId();
  const inputRef = useRef<HTMLInputElement>(null);

  // Whatever had the focus gets it back when the palette closes.
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    inputRef.current?.focus();
    return () => {
      if (before && document.contains(before)) before.focus();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    api
      .getMemories()
      .then((r) => {
        if (!cancelled) setMemories({ status: 'ready', items: r.memories });
      })
      .catch(() => {
        // Said in the group, not drawn as "no memory matches".
        if (!cancelled) setMemories({ status: 'error', items: [] });
      });
    return () => {
      cancelled = true;
    };
  }, [api]);

  const q = query.trim().toLowerCase();
  const typed = query.trim();

  // What was said is searched once typing pauses. A reply to a query that has
  // since changed is dropped, so a slow answer never replaces a newer one.
  useEffect(() => {
    if (!typed) {
      setSaid({ status: 'idle', hits: [], total: 0 });
      return;
    }
    let cancelled = false;
    setSaid((s) => ({ ...s, status: 'loading' }));
    const timer = setTimeout(() => {
      api
        .searchChat(typed, PER_GROUP)
        .then((r) => {
          if (!cancelled) setSaid({ status: 'ready', hits: r.results, total: r.total });
        })
        .catch(() => {
          // Said in the group, not drawn as "nothing matches".
          if (!cancelled) setSaid({ status: 'error', hits: [], total: 0 });
        });
    }, SEARCH_PAUSE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [api, typed]);

  const groups = useMemo(() => {
    const agentName = (id: string) => agents.find((a) => a.id === id)?.name ?? id;
    // Leaving for a screen leaves the living room the way the sidebar does.
    const go = (path: string) => {
      leaveDraft();
      setSelectedAgentId(null);
      setSystemActive(false);
      navigate(path);
    };

    const screens: Item[] = [
      { key: 's:agents', group: 'screens' as const, label: t('agent'), run: () => go('/') },
      { key: 's:cli', group: 'screens' as const, label: ta('cli_agent.page_title'), run: () => go('/agents/cli') },
      { key: 's:mcp', group: 'screens' as const, label: t('mcp'), run: () => go('/mcp-servers') },
      { key: 's:memory', group: 'screens' as const, label: t('memory'), run: () => go('/dashboard') },
      { key: 's:cron', group: 'screens' as const, label: t('cron'), run: () => go('/cron') },
      ...SETTINGS_SECTIONS.map(
        (s): Item => ({
          key: `s:settings:${s}`,
          group: 'screens' as const,
          label: `${t('settings')}: ${ts(`sections.${s}`)}`,
          run: () => go(`/settings?section=${s}`),
        }),
      ),
    ].filter((i) => !q || includes(i.label, q));

    const convs: Item[] = conversations
      .map(
        (c): Item => ({
          key: `c:${c.id}`,
          group: 'conversations',
          label: displayTitle(c, t('untitled_conversation')),
          detail: agentName(c.agent_id),
          run: () => open(c.agent_id, c.id),
        }),
      )
      .filter((i) => !q || includes(i.label, q) || includes(i.detail, q));

    const messages: Item[] =
      q && said.status === 'ready'
        ? said.hits.flatMap((h): Item[] => {
            const conversationId = h.conversation_id;
            if (!conversationId) return [];
            const where = {
              title: h.conversation_title?.trim() ? h.conversation_title : t('untitled_conversation'),
              agent: agentName(h.agent_id),
            };
            return [
              {
                key: `t:${h.message_id}`,
                group: 'messages',
                label: h.snippet,
                detail: h.archived ? t('palette.said_in_archived', where) : t('palette.said_in', where),
                run: () => open(h.agent_id, conversationId),
              },
            ];
          })
        : [];

    const srv: Item[] = q
      ? servers
          .map(
            (s): Item => ({
              key: `m:${s.id}`,
              group: 'servers',
              label: displayServerId(s.id),
              detail: s.description,
              run: () => go(`/mcp-servers?server=${encodeURIComponent(s.id)}`),
            }),
          )
          .filter((i) => includes(i.label, q) || includes(i.detail, q))
      : [];

    const mems: Item[] = q
      ? memories.items
          .filter((m) => includes(m.content, q))
          .map(
            (m): Item => ({
              key: `r:${m.id}`,
              group: 'memories',
              label: m.content.length > 90 ? `${m.content.slice(0, 90)}…` : m.content,
              detail: agentName(m.agent_id),
              run: () => go(`/dashboard?q=${encodeURIComponent(query.trim())}`),
            }),
          )
      : [];

    return (
      [
        ['screens', screens, screens.length],
        ['conversations', convs, convs.length],
        // The kernel's count, not the page's: the page is at most PER_GROUP.
        ['messages', messages, Math.max(said.total, messages.length)],
        ['servers', srv, srv.length],
        ['memories', mems, mems.length],
      ] as [Group, Item[], number][]
    ).map(([group, items, total]) => ({ group, items: items.slice(0, PER_GROUP), total }));
  }, [
    q,
    query,
    said,
    conversations,
    servers,
    memories,
    agents,
    t,
    ts,
    ta,
    open,
    leaveDraft,
    setSelectedAgentId,
    setSystemActive,
    navigate,
  ]);

  const flat = groups.flatMap((g) => g.items);
  const clamped = flat.length === 0 ? -1 : Math.min(active, flat.length - 1);

  useEffect(() => {
    setActive(0);
  }, [q]);

  const choose = (item: Item | undefined) => {
    if (!item) return;
    onClose();
    item.run();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === 'ArrowDown' && flat.length > 0) {
      e.preventDefault();
      setActive((clamped + 1) % flat.length);
    } else if (e.key === 'ArrowUp' && flat.length > 0) {
      e.preventDefault();
      setActive((clamped - 1 + flat.length) % flat.length);
    } else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
      e.preventDefault();
      choose(flat[clamped]);
    } else if (e.key === 'Tab') {
      // The field is the palette's one stop; Tab does not wander behind it.
      e.preventDefault();
    }
  };

  const groupTitle = (g: Group) => (g === 'memories' ? t('palette.group_memories_recent') : t(`palette.group_${g}`));

  const optionId = (key: string) => `${listId}-${key}`;
  let index = -1;

  return (
    <div
      className="palette-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="palette" role="dialog" aria-modal="true" aria-label={t('palette.title')}>
        <input
          ref={inputRef}
          role="combobox"
          aria-expanded="true"
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={clamped >= 0 ? optionId(flat[clamped].key) : undefined}
          aria-label={t('palette.title')}
          placeholder={t('palette.placeholder')}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
        />
        <div className="palette-list" id={listId} role="listbox" aria-label={t('palette.title')}>
          {groups.map(({ group, items, total }) =>
            items.length === 0 &&
            !(group === 'memories' && q && memories.status === 'error') &&
            !(group === 'messages' && q && said.status === 'error') ? null : (
              // A group of options inside the listbox: <fieldset> is a form control's group, not this.
              // biome-ignore lint/a11y/useSemanticElements: ARIA group within a listbox
              <div role="group" aria-label={groupTitle(group)} key={group} data-testid={`palette-${group}`}>
                <div className="pg">
                  {groupTitle(group)}
                  {total > items.length && (
                    <span className="more">{t('palette.more', { count: total - items.length })}</span>
                  )}
                </div>
                {group === 'memories' && memories.status === 'error' && (
                  <div className="pnote">{t('palette.memories_failed')}</div>
                )}
                {group === 'messages' && said.status === 'error' && (
                  <div className="pnote">{t('palette.said_failed')}</div>
                )}
                {items.map((item) => {
                  index += 1;
                  const on = index === clamped;
                  const mine = index;
                  return (
                    <div
                      key={item.key}
                      id={optionId(item.key)}
                      role="option"
                      tabIndex={-1}
                      aria-selected={on}
                      className={on ? 'po on' : 'po'}
                      onMouseMove={() => setActive(mine)}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => choose(item)}
                    >
                      <span className="l">{item.label}</span>
                      {item.detail && <span className="d">{item.detail}</span>}
                    </div>
                  );
                })}
              </div>
            ),
          )}
          {flat.length === 0 && !(q && memories.status === 'error') && !(q && said.status !== 'ready') && (
            <div className="pnote">{t('palette.no_match')}</div>
          )}
        </div>
      </div>
    </div>
  );
}
