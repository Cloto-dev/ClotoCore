import { useEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useAgentCreation } from '../../hooks/useAgentCreation';
import { useMcpServers } from '../../hooks/useMcpServers';
import { displayServerId } from '../../lib/format';
import { isEngineServer, isMemoryServer } from '../../lib/serverCategory';
import { Select } from '../ui/Select';

interface Props {
  onClose: () => void;
  onCreated: () => void;
}

/**
 * Making a new agent, as a question asked over the roster.
 *
 * Four fields are enough to have someone to talk to; everything else an agent
 * can be given is on its settings page, where it can be changed later. The
 * password and the routing rules are folded away because neither is needed to
 * start, and a first-run form that opens with six empty controls teaches that
 * this is complicated.
 */
export function CreateAgentModal({ onClose, onCreated }: Props) {
  const { t } = useTranslation('agents');
  const { t: tc } = useTranslation('common');
  const { servers } = useMcpServers();
  const nameRef = useRef<HTMLInputElement>(null);

  const engines = useMemo(() => servers.filter((s) => isEngineServer(s) && s.status === 'Connected'), [servers]);
  const memories = useMemo(() => servers.filter((s) => isMemoryServer(s) && s.status === 'Connected'), [servers]);

  const {
    form,
    updateField,
    handleCreate,
    isCreating,
    createError,
    addRoutingRule,
    updateRoutingRule,
    removeRoutingRule,
  } = useAgentCreation(onCreated);

  useEffect(() => {
    nameRef.current?.focus();
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) onClose();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [onClose]);

  const canCreate = Boolean(form.name && form.desc && form.engine) && !isCreating;

  return (
    // The backdrop is the pointer's way out; Escape is the keyboard's, bound above.
    <div
      className="wmodal-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="ws wmodal" role="dialog" aria-modal="true" aria-label={t('create_agent')}>
        <h2>{t('create_agent')}</h2>

        <div className="frow">
          <div className="k">{t('form.name')}</div>
          <div className="v">
            <input
              ref={nameRef}
              className="in"
              aria-label={t('form.name')}
              placeholder={t('form.name_placeholder')}
              value={form.name}
              onChange={(e) => updateField('name', e.target.value)}
            />
          </div>
        </div>

        <div className="frow">
          <div className="k">
            {t('form.description')}
            <small>{t('create.description_sub')}</small>
          </div>
          <div className="v">
            <textarea
              className="in"
              aria-label={t('form.description')}
              placeholder={t('form.desc_placeholder')}
              value={form.desc}
              onChange={(e) => updateField('desc', e.target.value)}
            />
          </div>
        </div>

        <div className="frow">
          <div className="k">{t('form.llm_engine')}</div>
          <div className="v">
            {engines.length > 0 ? (
              <Select
                label={t('form.llm_engine')}
                placeholder={t('form.select')}
                value={form.engine}
                options={engines.map((s) => ({ value: s.id, label: displayServerId(s.id) }))}
                onChange={(v) => updateField('engine', v)}
              />
            ) : (
              <div className="hint">{t('form.no_engines')}</div>
            )}
          </div>
        </div>

        <div className="frow">
          <div className="k">{t('form.memory')}</div>
          <div className="v">
            <Select
              label={t('form.memory')}
              placeholder={t('form.memory_none')}
              value={form.memory}
              options={[
                { value: '', label: t('form.memory_none') },
                ...memories.map((s) => ({ value: s.id, label: displayServerId(s.id) })),
              ]}
              onChange={(v) => updateField('memory', v)}
            />
          </div>
        </div>

        <details>
          <summary>{t('create.advanced')}</summary>

          <div className="frow">
            <div className="k">
              {t('form.password')}
              <small>{t('create.password_sub')}</small>
            </div>
            <div className="v">
              <input
                className="in"
                type="password"
                aria-label={t('form.password')}
                placeholder={t('form.password_placeholder')}
                value={form.password}
                onChange={(e) => updateField('password', e.target.value)}
              />
            </div>
          </div>

          <div className="frow">
            <div className="k">
              {t('routing.title')}
              <small>{t('create.routing_sub')}</small>
            </div>
            <div className="v">
              {form.routingRules.map((rule, i) => (
                // Keyed by position: a rule has no id of its own, and two rules
                // may hold the same text while being edited.
                <div className="tool rule" key={i}>
                  <input
                    className="in mono"
                    aria-label={t('routing.match_label', { index: i + 1 })}
                    placeholder="contains:keyword"
                    value={rule.match}
                    onChange={(e) => updateRoutingRule(i, 'match', e.target.value)}
                  />
                  <Select
                    label={t('routing.engine_label', { index: i + 1 })}
                    placeholder={t('routing.select_engine')}
                    value={rule.engine}
                    options={engines.map((s) => ({ value: s.id, label: displayServerId(s.id) }))}
                    onChange={(v) => updateRoutingRule(i, 'engine', v)}
                  />
                  <button
                    type="button"
                    className="btn"
                    aria-label={t('routing.remove_rule')}
                    onClick={() => removeRoutingRule(i)}
                  >
                    ×
                  </button>
                </div>
              ))}
              <button type="button" className="btn" onClick={addRoutingRule}>
                {t('routing.add_rule')}
              </button>
              <div className="hint">{t('routing.help')}</div>
            </div>
          </div>
        </details>

        {createError && <div className="hint danger">{createError}</div>}

        <div className="acts">
          <span className="spacer" />
          <button type="button" className="btn" onClick={onClose} disabled={isCreating}>
            {tc('cancel')}
          </button>
          <button type="button" className="btn pri" onClick={handleCreate} disabled={!canCreate}>
            {isCreating ? tc('loading') : tc('create')}
          </button>
        </div>
      </div>
    </div>
  );
}
