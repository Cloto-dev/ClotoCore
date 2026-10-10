import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as THREE from 'three';
import { useAgentContext } from '../contexts/AgentContext';
import { useApi } from '../hooks/useApi';
import type { AgentMetadata } from '../types';
import { VrmModelLoader } from '../vrm/engine/VrmModelLoader';
import { AssetFileError, validateIconFile, validateModelFile } from './assetFiles';
import { partnerMediaChanged } from './usePartnerMedia';

// HARDCODED(dashboard/src/pages/AgentSettingsPage.tsx::DEFAULT_AGENT_ID): this screen honors the same protected built-in partner.
const DEFAULT_AGENT_ID = 'agent.cloto_default';

export function PartnerSettings({
  agent,
  onBack,
  onAdvanced,
}: {
  agent: AgentMetadata;
  onBack: () => void;
  onAdvanced?: () => void;
}) {
  const { t } = useTranslation('agents');
  const api = useApi();
  const { refetchAgents } = useAgentContext();
  const [name, setName] = useState(agent.name);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const locked = agent.id === DEFAULT_AGENT_ID;
  const working = useRef(false);
  useEffect(() => {
    setName(agent.name);
  }, [agent.id, agent.name]);

  async function change(operation: () => Promise<void>) {
    if (working.current) return;
    working.current = true;
    setBusy(true);
    setProblem(null);
    setSaved(false);
    try {
      await operation();
      partnerMediaChanged(agent.id);
      await refetchAgents();
      setSaved(true);
    } catch (e) {
      setProblem(e instanceof AssetFileError ? t(`mate.file_${e.code}`) : t('mate.save_failed'));
    } finally {
      working.current = false;
      setBusy(false);
    }
  }

  async function choose(file: File, kind: 'icon' | 'vrm') {
    await change(async () => {
      if (kind === 'icon') {
        await validateIconFile(file);
        await api.uploadAvatar(agent.id, file);
      } else {
        await validateModelFile(file, 'vrm');
        // Parse successfully before replacing the stored model. The trial model is always disposed.
        const loader = new VrmModelLoader(new THREE.Scene());
        try {
          await loader.loadFile(file);
        } finally {
          loader.dispose();
        }
        await api.uploadVrm(agent.id, file);
      }
    });
  }

  return (
    <section className="mate-settings" aria-label={t('mate.partner_settings')}>
      <h2>{t('mate.partner_settings')}</h2>
      <p>{t('mate.assets_immediate')}</p>
      {locked && <p className="mate-notice">{t('mate.built_in')}</p>}
      <div className="mate-field">
        <label htmlFor="mate-name">{t('mate.name')}</label>
        <input
          id="mate-name"
          value={name}
          maxLength={20}
          disabled={busy || locked}
          onChange={(e) => {
            setName(e.target.value);
            setSaved(false);
          }}
        />
        <button
          type="button"
          disabled={busy || locked || !name.trim() || name.trim() === agent.name}
          onClick={() => void change(() => api.updateAgent(agent.id, { name: name.trim() }))}
        >
          {t('mate.apply_name')}
        </button>
      </div>
      <div className="mate-field">
        <h3>{t('mate.icon')}</h3>
        <p>{t('mate.icon_hint')}</p>
        <label className={`mate-upload ${locked || busy ? 'mate-disabled' : ''}`}>
          {t('mate.choose_icon')}
          <input
            type="file"
            accept="image/png,image/jpeg,image/webp"
            disabled={busy || locked}
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = '';
              if (file) void choose(file, 'icon');
            }}
          />
        </label>
        {agent.metadata?.has_avatar === 'true' && (
          <button type="button" disabled={busy || locked} onClick={() => void change(() => api.deleteAvatar(agent.id))}>
            {t('mate.remove_icon')}
          </button>
        )}
      </div>
      <div className="mate-field">
        <h3>{t('mate.vrm')}</h3>
        <p>{t('mate.vrm_hint')}</p>
        <label className={`mate-upload ${busy ? 'mate-disabled' : ''}`}>
          {t('mate.choose_vrm')}
          <input
            type="file"
            accept=".vrm"
            disabled={busy}
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = '';
              if (file) void choose(file, 'vrm');
            }}
          />
        </label>
        {agent.metadata?.has_vrm === 'true' && (
          <button type="button" disabled={busy} onClick={() => void change(() => api.deleteVrm(agent.id))}>
            {t('mate.remove_vrm')}
          </button>
        )}
      </div>
      {problem && (
        <p role="alert" className="mate-problem">
          {problem}
        </p>
      )}
      <p role="status">{busy ? t('mate.saving') : saved ? t('mate.saved') : ''}</p>
      {onAdvanced && (
        <button type="button" disabled={busy} onClick={onAdvanced}>
          {t('mate.advanced_settings')}
        </button>
      )}
      <button type="button" disabled={busy} onClick={onBack}>
        {t('mate.back')}
      </button>
    </section>
  );
}
