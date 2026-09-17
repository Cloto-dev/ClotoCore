import { Archive, MoreHorizontal, Pencil, Plus, Trash2 } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation } from 'react-router-dom';
import { useAgentContext } from '../contexts/AgentContext';
import { useConversations } from '../contexts/ConversationContext';
import { displayTitle, groupConversations } from '../lib/conversations';
import type { Conversation } from '../types';
import { ConfirmDialog } from './ui/ConfirmDialog';

/**
 * The sidebar's conversation list (docs/CONVERSATIONS_DESIGN.md §2d): New
 * chat, then every live conversation grouped by the day it was last active,
 * with the agent's name on each row. Rename, archive and delete live behind
 * the row's menu; delete asks first.
 */
export function ConversationList({ collapsed }: { collapsed: boolean }) {
  const { t } = useTranslation('nav');
  const location = useLocation();
  const { agents, selectedAgentId, systemActive, setSelectedAgentId, setSystemActive } = useAgentContext();
  const { conversations, openFor, open, newChat, rename, archive, remove } = useConversations();
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
  const agentName = (id: string) => agents.find((a) => a.id === id)?.name ?? id;
  const isAgentPage = location.pathname === '/';

  const handleNewChat = async () => {
    // A chat needs someone to talk to: with an agent open, start theirs;
    // otherwise go to the agents page to pick one.
    const agentId = selectedAgentId ?? (agents.length === 1 ? agents[0].id : null);
    if (!agentId) {
      setSelectedAgentId(null);
      setSystemActive(false);
      return;
    }
    const created = await newChat(agentId);
    open(agentId, created.id);
  };

  return (
    <div className="flex flex-col min-h-0">
      <button
        onClick={handleNewChat}
        title={collapsed ? t('new_chat') : undefined}
        aria-label={t('new_chat')}
        className={`relative mx-2 flex items-center ${collapsed ? 'justify-center px-0' : 'gap-2.5 px-3'} py-2 rounded-lg text-xs font-bold text-content-secondary hover:text-content-primary hover:bg-surface-field`}
      >
        <Plus size={collapsed ? 24 : 16} className="shrink-0" />
        {!collapsed && t('new_chat')}
      </button>

      {!collapsed && groups.length > 0 && (
        <div className="flex-1 min-h-0 overflow-y-auto px-2 mt-1" data-testid="conversation-list">
          {groups.map(({ group, items }) => (
            <div key={group} className="mb-2">
              <div className="px-3 pt-2 pb-1 text-xs text-content-tertiary">{t(`group_${group}`)}</div>
              {items.map((c) => {
                const isOpen =
                  isAgentPage && !systemActive && selectedAgentId === c.agent_id && openFor(c.agent_id) === c.id;
                return (
                  <ConversationRow
                    key={c.id}
                    conversation={c}
                    agentName={agentName(c.agent_id)}
                    isOpen={isOpen}
                    menuOpen={menuFor === c.id}
                    renaming={renaming?.id === c.id ? renaming.title : null}
                    onOpen={() => open(c.agent_id, c.id)}
                    onMenu={() => setMenuFor(menuFor === c.id ? null : c.id)}
                    onRenameStart={() => {
                      setMenuFor(null);
                      setRenaming({ id: c.id, title: c.title });
                    }}
                    onRenameChange={(title) => setRenaming({ id: c.id, title })}
                    onRenameCommit={async () => {
                      const title = renaming?.title.trim() ?? '';
                      setRenaming(null);
                      if (title && title !== c.title) await rename(c.agent_id, c.id, title);
                    }}
                    onRenameCancel={() => setRenaming(null)}
                    onArchive={async () => {
                      setMenuFor(null);
                      await archive(c.agent_id, c.id, true);
                    }}
                    onDelete={() => {
                      setMenuFor(null);
                      setDeleting(c);
                    }}
                  />
                );
              })}
            </div>
          ))}
        </div>
      )}

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
    </div>
  );
}

function ConversationRow({
  conversation,
  agentName,
  isOpen,
  menuOpen,
  renaming,
  onOpen,
  onMenu,
  onRenameStart,
  onRenameChange,
  onRenameCommit,
  onRenameCancel,
  onArchive,
  onDelete,
}: {
  conversation: Conversation;
  agentName: string;
  isOpen: boolean;
  menuOpen: boolean;
  renaming: string | null;
  onOpen: () => void;
  onMenu: () => void;
  onRenameStart: () => void;
  onRenameChange: (title: string) => void;
  onRenameCommit: () => void;
  onRenameCancel: () => void;
  onArchive: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation('nav');
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (renaming !== null) inputRef.current?.focus();
  }, [renaming]);
  const title = displayTitle(conversation, t('untitled_conversation'));

  if (renaming !== null) {
    return (
      <input
        ref={inputRef}
        aria-label={t('rename')}
        value={renaming}
        onChange={(e) => onRenameChange(e.target.value)}
        onBlur={onRenameCommit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') onRenameCommit();
          if (e.key === 'Escape') onRenameCancel();
        }}
        className="w-full px-3 py-1.5 rounded text-xs bg-surface-field text-content-primary border border-edge"
      />
    );
  }

  return (
    <div
      className={`group relative flex items-center rounded-lg ${isOpen ? 'bg-surface-primary' : 'hover:bg-surface-field'}`}
    >
      <button
        onClick={onOpen}
        title={title}
        aria-current={isOpen ? 'true' : undefined}
        className={`flex-1 min-w-0 flex items-baseline gap-2 px-3 py-1.5 text-left text-xs ${isOpen ? 'text-content-primary' : 'text-content-secondary'}`}
      >
        <span className="flex-1 min-w-0 truncate">{title}</span>
        <span className={`shrink-0 text-xs ${isOpen ? 'text-agent' : 'text-content-tertiary'}`}>{agentName}</span>
      </button>
      <button
        onClick={onMenu}
        aria-label={t('conversation_menu')}
        aria-expanded={menuOpen}
        className={`shrink-0 w-6 h-6 mr-1 rounded flex items-center justify-center text-content-tertiary hover:text-content-primary ${menuOpen ? '' : 'opacity-0 group-hover:opacity-100 focus:opacity-100'}`}
      >
        <MoreHorizontal size={14} />
      </button>
      {menuOpen && (
        <div
          role="menu"
          className="absolute right-1 top-full z-20 mt-1 min-w-[140px] rounded-lg border border-edge bg-surface-primary py-1 text-xs"
        >
          <MenuItem icon={Pencil} label={t('rename')} onClick={onRenameStart} />
          <MenuItem icon={Archive} label={t('archive')} onClick={onArchive} />
          <MenuItem icon={Trash2} label={t('delete')} onClick={onDelete} danger />
        </div>
      )}
    </div>
  );
}

function MenuItem({
  icon: Icon,
  label,
  onClick,
  danger = false,
}: {
  icon: typeof Pencil;
  label: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      role="menuitem"
      onClick={onClick}
      className={`w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-surface-field ${danger ? 'text-red-400' : 'text-content-secondary'}`}
    >
      <Icon size={12} />
      {label}
    </button>
  );
}
