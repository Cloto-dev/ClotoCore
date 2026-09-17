import type React from 'react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { useAgentContext } from '../contexts/AgentContext';
import { useConnection } from '../contexts/ConnectionContext';
import { useConversations } from '../contexts/ConversationContext';
import { useApi } from '../hooks/useApi';
import { useModules } from '../hooks/useModules';
import { useShortcut } from '../hooks/useShortcut';
import { displayTitle, groupConversations } from '../lib/conversations';
import type { Conversation } from '../types';
import { NotificationBell } from './NotificationBell';
import { requestShutdown } from './ShutdownOverlay';
import { ConfirmDialog } from './ui/ConfirmDialog';
import './AppSidebar.css';

/**
 * The sidebar of docs/gui/samples/01-chat-empty.html: the app name with search
 * and notifications, New chat, the conversations by day with the agent's name
 * on each row, the five destinations, and the kernel's state. Markup and CSS
 * follow the mock; only the data behind them is live.
 *
 * Kept beyond the mock, because the product still has them: the cron page and
 * any runtime modules (under the same nav style) and the shutdown control (in
 * the footer). The experimental-build mark sits at the window's bottom right.
 */

const ICONS = {
  search: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </svg>
  ),
  newChat: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </svg>
  ),
  agents: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <circle cx="9" cy="8" r="3.5" />
      <path d="M2 20a7 7 0 0 1 14 0M16 4a3.5 3.5 0 0 1 0 7M22 20a6 6 0 0 0-4-5.7" />
    </svg>
  ),
  mcp: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <rect x="3" y="4" width="18" height="7" rx="1.5" />
      <rect x="3" y="13" width="18" height="7" rx="1.5" />
    </svg>
  ),
  memory: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M9 3a3 3 0 0 0-3 3v1a3 3 0 0 0-2 5 3 3 0 0 0 2 5v1a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3ZM15 3a3 3 0 0 1 3 3v1a3 3 0 0 1 2 5 3 3 0 0 1-2 5v1a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3Z" />
    </svg>
  ),
  settings: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1" />
    </svg>
  ),
  cron: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </svg>
  ),
  module: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <rect x="3" y="3" width="8" height="8" rx="1.5" />
      <rect x="13" y="3" width="8" height="8" rx="1.5" />
      <rect x="3" y="13" width="8" height="8" rx="1.5" />
      <rect x="13" y="13" width="8" height="8" rx="1.5" />
    </svg>
  ),
  help: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <circle cx="12" cy="12" r="9" />
      <path d="M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.7.4-1 1-1 1.7" />
      <path d="M12 17h.01" />
    </svg>
  ),
  power: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <path d="M12 3v9" />
      <path d="M6.3 6.3a8 8 0 1 0 11.4 0" />
    </svg>
  ),
  more: (
    <svg viewBox="0 0 24 24" fill="currentColor">
      <circle cx="5" cy="12" r="1.6" />
      <circle cx="12" cy="12" r="1.6" />
      <circle cx="19" cy="12" r="1.6" />
    </svg>
  ),
};

/** Groups the mock draws; everything older sits behind "show more". */
const VISIBLE_GROUPS = new Set(['today', 'yesterday', 'previous_7_days']);

interface AppSidebarProps {
  onSettingsClick: () => void;
  /** Open the help. Optional: a caller with no help to show gets no link. */
  onHelpClick?: () => void;
  /** Open search (⌘K). Optional: without it the button is not drawn. */
  onSearchClick?: () => void;
}

export const AppSidebar: React.FC<AppSidebarProps> = ({ onSettingsClick, onHelpClick, onSearchClick }) => {
  const navigate = useNavigate();
  const location = useLocation();
  const { t } = useTranslation('nav');
  const { t: tSettings } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');
  const { connected } = useConnection();

  // The updater announces a newer build with this event; the foot's version
  // number turns into the way to it.
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  useEffect(() => {
    const handler = (e: Event) => setUpdateVersion((e as CustomEvent).detail?.version ?? 'new');
    window.addEventListener('cloto-update-available', handler);
    return () => window.removeEventListener('cloto-update-available', handler);
  }, []);
  const api = useApi();
  const { agents, selectedAgentId, setSelectedAgentId, systemActive, setSystemActive, processingAgentIds } =
    useAgentContext();
  const { conversations, openFor, open, draft, startDraft, leaveDraft, rename, archive, remove } = useConversations();
  const { modules } = useModules();
  const [shutdownConfirm, setShutdownConfirm] = useState(false);
  const [showOlder, setShowOlder] = useState(false);
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; title: string } | null>(null);
  const [deleting, setDeleting] = useState<Conversation | null>(null);
  const [now, setNow] = useState(() => Date.now());

  // The day boundaries move: re-group once a minute so "today" stays true.
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, []);

  const groups = useMemo(() => groupConversations(conversations, now), [conversations, now]);
  const hiddenCount = groups.filter((g) => !VISIBLE_GROUPS.has(g.group)).reduce((n, g) => n + g.items.length, 0);
  // With nothing from the last week, folding the rest away leaves an empty list
  // and a lone "show more": the older conversations are then the list.
  const nothingRecent = hiddenCount === groups.reduce((n, g) => n + g.items.length, 0);
  const shownGroups = showOlder || nothingRecent ? groups : groups.filter((g) => VISIBLE_GROUPS.has(g.group));
  const agentName = (id: string) => agents.find((a) => a.id === id)?.name ?? id;
  const isAgentPage = location.pathname === '/';
  // The living room is showing: a conversation, or the new chat (which may be
  // turned to "create an agent", with nobody selected).
  const isChatOpen = isAgentPage && !systemActive && (selectedAgentId !== null || draft !== null);
  const isNavActive = (path: string) => location.pathname === path;

  const handleNewChat = () => {
    // The new chat opens on whoever is present, else whoever was spoken with
    // last, else anyone who is on — and on "create an agent" when nobody
    // exists. Nothing is created here: the conversation exists, and appears in
    // this list, once something is said.
    const agentId =
      selectedAgentId ?? conversations[0]?.agent_id ?? (agents.find((a) => a.enabled) ?? agents[0])?.id ?? null;
    startDraft(agentId);
  };

  // ⌘N / Ctrl+N: the shortcut the mock prints beside New chat.
  useShortcut('newChat', () => {
    handleNewChat();
  });

  const goAgents = () => {
    leaveDraft();
    setSelectedAgentId(null);
    setSystemActive(false);
    navigate('/');
  };
  const goTo = (path: string) => {
    leaveDraft();
    setSelectedAgentId(null);
    setSystemActive(false);
    navigate(path);
  };

  const usableModules = modules.filter((m) => !m.error);

  return (
    <aside className="side" aria-label={t('sidebar')}>
      <div className="side-head">
        <span className="app">ClotoCore</span>
        {onSearchClick && (
          <button type="button" title={t('search')} aria-label={t('search')} onClick={onSearchClick}>
            {ICONS.search}
          </button>
        )}
        <NotificationBell />
      </div>

      <button type="button" className="newchat" onClick={handleNewChat} aria-label={t('new_chat')}>
        {ICONS.newChat}
        {t('new_chat')}
        <span className="kbd">⌘N</span>
      </button>

      <div className="convs" data-testid="conversation-list">
        {shownGroups.map(({ group, items }) => (
          <div key={group}>
            <div className="glabel">{t(`group_${group}`)}</div>
            {items.map((c) => {
              // While the new chat is open, no conversation is.
              const isOpen =
                isChatOpen && draft === null && selectedAgentId === c.agent_id && openFor(c.agent_id) === c.id;
              const mine = selectedAgentId === c.agent_id;
              const live = processingAgentIds.has(c.agent_id);
              const title = displayTitle(c, t('untitled_conversation'));
              if (renaming?.id === c.id) {
                return (
                  <RenameField
                    key={c.id}
                    value={renaming.title}
                    label={t('rename')}
                    onChange={(title) => setRenaming({ id: c.id, title })}
                    onCommit={async () => {
                      const next = renaming.title.trim();
                      setRenaming(null);
                      if (next && next !== c.title) await rename(c.agent_id, c.id, next);
                    }}
                    onCancel={() => setRenaming(null)}
                  />
                );
              }
              return (
                <div key={c.id} className="conv-row">
                  <button
                    type="button"
                    className={`conv${mine ? ' mine' : ''}${isOpen ? ' on' : ''}`}
                    title={title}
                    aria-current={isOpen ? 'true' : undefined}
                    onClick={() => open(c.agent_id, c.id)}
                  >
                    <span className="t">{title}</span>
                    {live && <span className="live" title={t('responding')} />}
                    <span className="who">{agentName(c.agent_id)}</span>
                  </button>
                  <button
                    type="button"
                    className="conv-menu-btn"
                    aria-label={t('conversation_menu')}
                    aria-expanded={menuFor === c.id}
                    onClick={() => setMenuFor(menuFor === c.id ? null : c.id)}
                  >
                    {ICONS.more}
                  </button>
                  {menuFor === c.id && (
                    <div role="menu" className="conv-menu">
                      <button
                        type="button"
                        role="menuitem"
                        onClick={() => {
                          setMenuFor(null);
                          setRenaming({ id: c.id, title: c.title });
                        }}
                      >
                        {t('rename')}
                      </button>
                      <button
                        type="button"
                        role="menuitem"
                        onClick={async () => {
                          setMenuFor(null);
                          await archive(c.agent_id, c.id, true);
                        }}
                      >
                        {t('archive')}
                      </button>
                      <button
                        type="button"
                        role="menuitem"
                        className="danger"
                        onClick={() => {
                          setMenuFor(null);
                          setDeleting(c);
                        }}
                      >
                        {t('delete')}
                      </button>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        ))}
        {hiddenCount > 0 && !nothingRecent && (
          <button type="button" className="more" onClick={() => setShowOlder((v) => !v)}>
            {showOlder ? t('show_less') : t('show_more')}
          </button>
        )}
      </div>

      <nav className="nav">
        <button
          type="button"
          className={`navlink${isAgentPage && !isChatOpen && !systemActive ? ' on' : ''}`}
          onClick={goAgents}
        >
          {ICONS.agents}
          {t('agent')}
        </button>
        <button
          type="button"
          className={`navlink${isNavActive('/mcp-servers') ? ' on' : ''}`}
          onClick={() => goTo('/mcp-servers')}
        >
          {ICONS.mcp}
          {t('mcp')}
        </button>
        <button
          type="button"
          className={`navlink${isNavActive('/dashboard') ? ' on' : ''}`}
          onClick={() => goTo('/dashboard')}
        >
          {ICONS.memory}
          {t('memory')}
        </button>
        <button type="button" className={`navlink${isNavActive('/settings') ? ' on' : ''}`} onClick={onSettingsClick}>
          {ICONS.settings}
          {t('settings')}
        </button>
        <button type="button" className={`navlink${isNavActive('/cron') ? ' on' : ''}`} onClick={() => goTo('/cron')}>
          {ICONS.cron}
          {t('cron')}
        </button>
        {usableModules.map((module) => {
          const path = `/modules/${module.id}`;
          return (
            <button
              type="button"
              key={module.id}
              className={`navlink${isNavActive(path) ? ' on' : ''}`}
              onClick={() => goTo(path)}
            >
              {ICONS.module}
              {module.name || module.id}
            </button>
          );
        })}
        {onHelpClick && (
          <button type="button" className="navlink" onClick={onHelpClick}>
            {ICONS.help}
            {t('help')}
          </button>
        )}
      </nav>

      <div className="side-foot">
        <span className={connected ? 'ok' : 'bad'}>{connected ? t('kernel_running') : t('kernel_unreachable')}</span>
        <span>{t('agents_count', { count: agents.length })}</span>
        {updateVersion ? (
          // A newer build exists: the version itself is the way to it.
          <button
            type="button"
            className="num update"
            title={tCommon('update_available_banner', { version: updateVersion })}
            aria-label={tCommon('update_available_banner', { version: updateVersion })}
            onClick={() =>
              window.dispatchEvent(new CustomEvent('cloto-open-settings', { detail: { section: 'about' } }))
            }
          >
            {__APP_VERSION__} ↑
          </button>
        ) : (
          <span className="num">{__APP_VERSION__}</span>
        )}
        <button
          type="button"
          className="power"
          onClick={() => setShutdownConfirm(true)}
          title={t('shutdown')}
          aria-label={t('shutdown')}
        >
          {ICONS.power}
        </button>
      </div>

      <ConfirmDialog
        open={shutdownConfirm}
        title={tSettings('advanced.shutdown_confirm_title')}
        message={tSettings('advanced.shutdown_confirm_message')}
        confirmLabel={tSettings('advanced.shutdown_confirm_label')}
        cancelLabel={tCommon('cancel')}
        variant="danger"
        onConfirm={() => {
          setShutdownConfirm(false);
          void requestShutdown(api.post);
        }}
        onCancel={() => setShutdownConfirm(false)}
      />
      <ConfirmDialog
        open={deleting !== null}
        title={t('delete_conversation_title')}
        message={t('delete_conversation_message', {
          title: deleting ? displayTitle(deleting, t('untitled_conversation')) : '',
        })}
        confirmLabel={t('delete')}
        cancelLabel={t('cancel')}
        variant="danger"
        onConfirm={async () => {
          const target = deleting;
          setDeleting(null);
          if (target) await remove(target.agent_id, target.id);
        }}
        onCancel={() => setDeleting(null)}
      />
    </aside>
  );
};

function RenameField({
  value,
  label,
  onChange,
  onCommit,
  onCancel,
}: {
  value: string;
  label: string;
  onChange: (v: string) => void;
  onCommit: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ref.current?.focus();
  }, []);
  return (
    <input
      ref={ref}
      className="conv-rename"
      aria-label={label}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      onBlur={onCommit}
      onKeyDown={(e) => {
        if (e.key === 'Enter') onCommit();
        if (e.key === 'Escape') onCancel();
      }}
    />
  );
}
