import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAgentContext } from '../../contexts/AgentContext';
import { useConversations } from '../../contexts/ConversationContext';
import { useUserIdentity } from '../../contexts/UserIdentityContext';
import { useApi } from '../../hooks/useApi';
import { displayTitle } from '../../lib/conversations';
import type { Conversation } from '../../types';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { SectionCard } from './common';

/**
 * Where archived conversations live (docs/CONVERSATIONS_DESIGN.md §2f): hidden
 * from the sidebar, kept whole, listed here per agent with the way back, and
 * the two bulk actions beside them. Delete-all asks first; archive-all does
 * not, because it can be undone one row at a time here.
 */
export function ConversationsSection() {
  const { t } = useTranslation('settings');
  const api = useApi();
  const { identity } = useUserIdentity();
  const { agents } = useAgentContext();
  const { refresh } = useConversations();
  const [archived, setArchived] = useState<Conversation[]>([]);
  const [confirmDeleteAll, setConfirmDeleteAll] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    const lists = await Promise.all(
      agents.map((a) => api.listConversations(a.id, identity.id, true).catch(() => [] as Conversation[])),
    );
    setArchived(
      lists
        .flat()
        .filter((c) => c.archived_at !== null)
        .sort((a, b) => (b.archived_at ?? 0) - (a.archived_at ?? 0)),
    );
  }, [agents, api, identity.id]);

  useEffect(() => {
    load();
  }, [load]);

  const agentName = (id: string) => agents.find((a) => a.id === id)?.name ?? id;

  const unarchive = async (c: Conversation) => {
    await api.updateConversation(c.agent_id, c.id, { archived: false });
    await Promise.all([load(), refresh()]);
  };
  const remove = async (c: Conversation) => {
    await api.deleteConversation(c.agent_id, c.id);
    await load();
  };
  const archiveAll = async (agentId: string) => {
    const { archived: n } = await api.archiveAllConversations(agentId, identity.id);
    setNotice(t('conversations.archived_count', { count: n }));
    await Promise.all([load(), refresh()]);
  };
  const deleteAll = async (agentId: string) => {
    const { deleted } = await api.deleteAllConversations(agentId, identity.id);
    setNotice(t('conversations.deleted_count', { count: deleted }));
    await Promise.all([load(), refresh()]);
  };

  return (
    <>
      <SectionCard title={t('conversations.archived_title')}>
        {archived.length === 0 ? (
          <p className="text-xs text-content-tertiary">{t('conversations.archived_empty')}</p>
        ) : (
          <ul className="divide-y divide-edge" data-testid="archived-conversations">
            {archived.map((c) => (
              <li key={c.id} className="flex items-center gap-3 py-2 text-xs">
                <span className="flex-1 min-w-0 truncate text-content-primary">
                  {displayTitle(c, t('conversations.untitled'))}
                </span>
                <span className="shrink-0 text-content-tertiary">{agentName(c.agent_id)}</span>
                <button
                  onClick={() => unarchive(c)}
                  className="shrink-0 px-2 py-1 rounded border border-edge text-content-secondary hover:text-content-primary hover:border-agent"
                >
                  {t('conversations.unarchive')}
                </button>
                <button
                  onClick={() => remove(c)}
                  className="shrink-0 px-2 py-1 rounded border border-edge text-content-tertiary hover:text-red-400 hover:border-red-500"
                >
                  {t('conversations.delete')}
                </button>
              </li>
            ))}
          </ul>
        )}
      </SectionCard>

      <SectionCard title={t('conversations.bulk_title')}>
        <p className="text-xs text-content-tertiary mb-3">{t('conversations.bulk_desc')}</p>
        <ul className="divide-y divide-edge">
          {agents.map((a) => (
            <li key={a.id} className="flex items-center gap-3 py-2 text-xs">
              <span className="flex-1 min-w-0 truncate text-content-primary">{a.name}</span>
              <button
                onClick={() => archiveAll(a.id)}
                className="shrink-0 px-2 py-1 rounded border border-edge text-content-secondary hover:text-content-primary hover:border-agent"
              >
                {t('conversations.archive_all')}
              </button>
              <button
                onClick={() => setConfirmDeleteAll(a.id)}
                className="shrink-0 px-2 py-1 rounded border border-edge text-content-tertiary hover:text-red-400 hover:border-red-500"
              >
                {t('conversations.delete_all')}
              </button>
            </li>
          ))}
        </ul>
        {notice && <p className="mt-3 text-xs text-content-secondary">{notice}</p>}
      </SectionCard>

      <ConfirmDialog
        open={confirmDeleteAll !== null}
        title={t('conversations.delete_all_title')}
        message={t('conversations.delete_all_message', {
          agent: confirmDeleteAll ? agentName(confirmDeleteAll) : '',
        })}
        confirmLabel={t('conversations.delete_all')}
        cancelLabel={t('conversations.cancel')}
        variant="danger"
        onConfirm={async () => {
          const target = confirmDeleteAll;
          setConfirmDeleteAll(null);
          if (target) await deleteAll(target);
        }}
        onCancel={() => setConfirmDeleteAll(null)}
      />
    </>
  );
}
