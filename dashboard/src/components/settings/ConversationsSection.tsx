import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAgentContext } from '../../contexts/AgentContext';
import { useConversations } from '../../contexts/ConversationContext';
import { useUserIdentity } from '../../contexts/UserIdentityContext';
import { useApi } from '../../hooks/useApi';
import { displayTitle } from '../../lib/conversations';
import type { Conversation } from '../../types';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { SettingsGroup, SettingsRow } from './common';

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
  // Whose list could not be read, and whether any read has finished. An agent
  // whose list failed is not an agent with nothing archived: until every list
  // has been read, this screen cannot say "nothing".
  const [unread, setUnread] = useState<string[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [confirmDeleteAll, setConfirmDeleteAll] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    const results = await Promise.allSettled(agents.map((a) => api.listConversations(a.id, identity.id, true)));
    const lists: Conversation[][] = [];
    const failed: string[] = [];
    results.forEach((r, i) => {
      if (r.status === 'fulfilled') lists.push(r.value);
      else failed.push(agents[i].id);
    });
    setUnread(failed);
    setLoaded(true);
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
      <SettingsGroup title={t('conversations.archived_title')}>
        {unread.length > 0 && (
          <p className="says warn" role="alert">
            {t('conversations.unread', { agents: unread.map(agentName).join(', ') })}{' '}
            <button type="button" className="btn" onClick={() => load()}>
              {t('conversations.retry')}
            </button>
          </p>
        )}
        {archived.length === 0 ? (
          // "Nothing is archived" is a claim about every agent; it is made only
          // once every list has been read.
          loaded && unread.length === 0 && <p className="gdesc">{t('conversations.archived_empty')}</p>
        ) : (
          <ul className="slist" data-testid="archived-conversations">
            {archived.map((c) => (
              <li key={c.id}>
                <span className="t">{displayTitle(c, t('conversations.untitled'))}</span>
                <span className="who">{agentName(c.agent_id)}</span>
                <button type="button" className="btn" onClick={() => unarchive(c)}>
                  {t('conversations.unarchive')}
                </button>
                <button type="button" className="btn danger" onClick={() => remove(c)}>
                  {t('conversations.delete')}
                </button>
              </li>
            ))}
          </ul>
        )}
      </SettingsGroup>

      <SettingsGroup title={t('conversations.bulk_title')}>
        <p className="gdesc">{t('conversations.bulk_desc')}</p>
        {agents.map((a) => (
          <SettingsRow key={a.id} label={a.name}>
            <button type="button" className="btn" onClick={() => archiveAll(a.id)}>
              {t('conversations.archive_all')}
            </button>
            <button type="button" className="btn danger" onClick={() => setConfirmDeleteAll(a.id)}>
              {t('conversations.delete_all')}
            </button>
          </SettingsRow>
        ))}
        {notice && <p className="says">{notice}</p>}
      </SettingsGroup>

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
