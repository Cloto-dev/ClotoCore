import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import type { AgentMetadata } from '../../types';

interface Props {
  agent: AgentMetadata;
  onClose: () => void;
  onDeleted: () => void;
}

/**
 * Deleting an agent, behind a confirm and — when the agent has one — its power
 * password.
 *
 * This is the exception to the deferred-save rule (CLAUDE.md, "Agent Config
 * Rules"): the modal is itself the chance to cancel, and a pending delete would
 * invent a "cancel the cancel" step that is easier to get wrong than this.
 */
export function DeleteAgentModal({ agent, onClose, onDeleted }: Props) {
  const api = useApi();
  const { t } = useTranslation('agents');
  const { t: tc } = useTranslation('common');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const needsPassword = agent.metadata?.has_power_password === 'true';

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) onClose();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [onClose]);

  const handleDeleteConfirm = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.deleteAgent(agent.id, needsPassword ? password : undefined);
      onDeleted();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Unknown error');
    } finally {
      setBusy(false);
    }
  };

  return (
    // The backdrop is the pointer's way out; Escape is the keyboard's, bound above.
    <div
      className="wmodal-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="ws wmodal" role="dialog" aria-modal="true" aria-label={t('delete.title')}>
        <h2 className="danger">{t('delete.title')}</h2>
        <div className="hint">{t('delete.warning')}</div>
        <div className="frow">
          <div className="k">
            {agent.name}
            <small>{agent.id}</small>
          </div>
          <div className="v">
            {needsPassword && (
              <input
                className="in"
                type="password"
                aria-label={t('delete.password_placeholder')}
                placeholder={t('delete.password_placeholder')}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            )}
            {error && <div className="hint danger">{error}</div>}
          </div>
        </div>
        <div className="acts">
          <span className="spacer" />
          <button type="button" className="btn" onClick={onClose} disabled={busy}>
            {tc('cancel')}
          </button>
          <button
            type="button"
            className="btn danger"
            onClick={handleDeleteConfirm}
            disabled={busy || (needsPassword && !password)}
          >
            {tc('delete')}
          </button>
        </div>
      </div>
    </div>
  );
}
