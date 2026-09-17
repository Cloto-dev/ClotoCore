import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { REPOSITORY_URL } from '../../constants';
import { useLocalStorage } from '../../hooks/useStorage';
import {
  applyUpdate,
  checkForUpdates,
  isTauri,
  UPDATE_CHANNEL_STORAGE_KEY,
  UPDATE_CHANNELS,
  type UpdateChannel,
  type UpdateInfo,
} from '../../lib/tauri';
import { SetupWizard } from '../SetupWizard';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { Select, SettingsGroup, SettingsRow, Toggle } from './common';

type UpdateState = 'idle' | 'checking' | 'up-to-date' | 'available' | 'updating' | 'updated' | 'error';

export function AboutSection() {
  const [updateState, setUpdateState] = useState<UpdateState>('idle');
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [error, setError] = useState('');
  const [updateOutput, setUpdateOutput] = useState('');
  const [showWizard, setShowWizard] = useState(false);
  const { t } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');
  const [autoUpdateRaw, setAutoUpdateRaw] = useLocalStorage('cloto-auto-update', 'on');
  const autoUpdateEnabled = autoUpdateRaw !== 'off';
  const [channelRaw, setChannelRaw] = useLocalStorage(UPDATE_CHANNEL_STORAGE_KEY, 'stable');
  const channel: UpdateChannel = UPDATE_CHANNELS.includes(channelRaw as UpdateChannel)
    ? (channelRaw as UpdateChannel)
    : 'stable';
  const [experimentalConfirm, setExperimentalConfirm] = useState(false);

  const applyChannel = (next: UpdateChannel) => {
    setChannelRaw(next);
    // A channel switch invalidates any previous check result.
    setUpdateInfo(null);
    setUpdateState('idle');
  };

  const handleChannelSelect = (next: UpdateChannel) => {
    if (next === channel) return;
    if (next === 'experimental') {
      // Switching TO experimental requires explicit confirmation (design §5.1).
      setExperimentalConfirm(true);
      return;
    }
    applyChannel(next);
  };

  const handleCheck = async () => {
    setUpdateState('checking');
    setError('');
    try {
      const info = await checkForUpdates();
      setUpdateInfo(info);
      setUpdateState(info.available ? 'available' : 'up-to-date');
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Failed to check for updates';
      setError(message);
      setUpdateState('error');
    }
  };

  const handleUpdate = async () => {
    setUpdateState('updating');
    setError('');
    try {
      const output = await applyUpdate();
      setUpdateOutput(output);
      setUpdateState('updated');
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Failed to apply update';
      setError(message);
      setUpdateState('error');
    }
  };

  const formatDate = (iso?: string) => {
    if (!iso) return '';
    try {
      return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
    } catch {
      return iso;
    }
  };

  return (
    <>
      <SettingsGroup title={t('about.clotocore')}>
        <SettingsRow label={t('about.version_label')} desc={t('about.description')}>
          <span className="val num">v{__APP_VERSION__}</span>
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup title={t('about.updates')}>
        {/* Auto-update check and the channel it reads — desktop shell only. */}
        {isTauri && (
          <SettingsRow label={t('about.auto_update')} desc={t('about.auto_update_desc')}>
            <Toggle
              label={t('about.auto_update')}
              checked={autoUpdateEnabled}
              onChange={() => setAutoUpdateRaw(autoUpdateEnabled ? 'off' : 'on')}
            />
          </SettingsRow>
        )}

        {isTauri && (
          <SettingsRow label={t('about.update_channel')} desc={t('about.update_channel_desc')}>
            <Select<UpdateChannel>
              label={t('about.update_channel')}
              value={channel}
              onChange={handleChannelSelect}
              options={[
                { value: 'stable', label: t('about.channel_stable'), hint: t('about.channel_stable_hint') },
                { value: 'current', label: t('about.channel_current'), hint: t('about.channel_current_hint') },
                {
                  value: 'experimental',
                  label: t('about.channel_experimental'),
                  hint: t('about.channel_experimental_hint'),
                },
              ]}
            />
          </SettingsRow>
        )}

        <SettingsRow label={t('about.latest_label')} desc={t('about.latest_desc')}>
          {updateState === 'checking' ? (
            <span className="val">{t('about.checking')}</span>
          ) : updateState === 'updating' ? (
            <span className="val">{t('about.applying')}</span>
          ) : (
            <button type="button" className="btn" onClick={handleCheck}>
              {updateState === 'idle' || updateState === 'error' ? t('about.check_for_updates') : t('about.recheck')}
            </button>
          )}
        </SettingsRow>

        {updateState === 'up-to-date' && (
          <p className="says ok">{t('about.up_to_date', { version: updateInfo?.currentVersion })}</p>
        )}

        {updateState === 'available' && updateInfo && (
          <div className="set-block">
            <p className="says">
              {t('about.available', { version: updateInfo.latestVersion })}
              {updateInfo.releaseDate && ` (${formatDate(updateInfo.releaseDate)})`}
            </p>
            {updateInfo.releaseNotes && (
              <p className="quote">
                {updateInfo.releaseNotes.slice(0, 500)}
                {updateInfo.releaseNotes.length > 500 && '...'}
              </p>
            )}
            {isTauri && (
              <button type="button" className="btn pri" onClick={handleUpdate}>
                {t('about.update_now')}
              </button>
            )}
          </div>
        )}

        {updateState === 'updated' && (
          <div className="set-block">
            <p className="says ok">{t('about.applied')}</p>
            {updateOutput && <p className="quote">{updateOutput.slice(0, 300)}</p>}
            <p className="says">{t('about.restart_hint')}</p>
          </div>
        )}

        {updateState === 'error' && error && <p className="says bad">{error}</p>}
      </SettingsGroup>

      <SettingsGroup title={t('about.license')}>
        <SettingsRow label={t('about.bsl')} desc={t('about.mit_convert')} />
      </SettingsGroup>

      <SettingsGroup title={t('about.links')}>
        <SettingsRow label={t('about.repository')}>
          <a className="val" href={REPOSITORY_URL} target="_blank" rel="noopener noreferrer">
            github.com/Cloto-dev/ClotoCore
          </a>
        </SettingsRow>
        <SettingsRow label={t('about.contact')}>
          <a className="val" href="mailto:ClotoCore@proton.me">
            ClotoCore@proton.me
          </a>
        </SettingsRow>
      </SettingsGroup>

      {/* Desktop shell only, for the same reason the first-run wizard is
          (see the gate in App.tsx): the wizard's preset step *replaces* the
          default agent's MCP grant set, and it cannot obtain the admin key
          outside Tauri, so over a browser this button led to a dead end at
          the key step — a broken affordance guarding a destructive one. */}
      {isTauri && (
        <SettingsGroup title={t('about.setup')}>
          <p className="gdesc">{t('about.setup_desc')}</p>
          <div className="set-block">
            <button type="button" className="btn" onClick={() => setShowWizard(true)}>
              {t('about.rerun_setup')}
            </button>
          </div>
        </SettingsGroup>
      )}

      {showWizard && (
        <SetupWizard
          onComplete={() => {
            setShowWizard(false);
            window.dispatchEvent(new CustomEvent('cloto-setup-rerun-complete'));
          }}
        />
      )}

      <ConfirmDialog
        open={experimentalConfirm}
        title={t('about.channel_confirm_title')}
        message={t('about.channel_confirm_message')}
        confirmLabel={t('about.channel_confirm_label')}
        cancelLabel={tCommon('cancel')}
        variant="danger"
        onConfirm={() => {
          setExperimentalConfirm(false);
          applyChannel('experimental');
        }}
        onCancel={() => setExperimentalConfirm(false)}
      />
    </>
  );
}
