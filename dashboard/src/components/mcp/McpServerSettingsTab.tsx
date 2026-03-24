import { RotateCcw, Save } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AlertCard } from '../../components/ui/AlertCard';
import { EnvVariableEditor } from '../../components/ui/EnvVariableEditor';
import { useApi } from '../../hooks/useApi';
import { useAsyncAction } from '../../hooks/useAsyncAction';
import { displayServerId } from '../../lib/format';
import type { DefaultPolicy, McpServerInfo, McpServerSettings } from '../../types';

interface Props {
  server: McpServerInfo;
  onRefresh: () => void;
}

export function McpServerSettingsTab({ server, onRefresh }: Props) {
  const api = useApi();
  const { t } = useTranslation('mcp');
  const [settings, setSettings] = useState<McpServerSettings | null>(null);
  const [defaultPolicy, setDefaultPolicy] = useState<DefaultPolicy>('opt-in');

  // Env editor state
  const [envEntries, setEnvEntries] = useState<{ key: string; value: string }[]>([]);
  const [initialEnvKeys, setInitialEnvKeys] = useState<Set<string>>(new Set());

  const saveAction = useAsyncAction('Failed to save settings');
  const loadAction = useAsyncAction('Failed to load settings');

  const loadSettings = useCallback(async () => {
    await loadAction.run(async () => {
      const data = await api.getMcpServerSettings(server.id);
      setSettings(data);
      setDefaultPolicy(data.default_policy);

      // Load env entries
      const env = data.env ?? {};
      const entries = Object.entries(env).map(([key, value]) => ({ key, value }));
      setEnvEntries(entries);
      setInitialEnvKeys(new Set(Object.keys(env)));
      setInitialEnvValues({ ...env });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server.id, api, loadAction.run]);

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  async function handleSave() {
    await saveAction.run(async () => {
      // Build env object from entries
      const envObj: Record<string, string> = {};
      for (const entry of envEntries) {
        if (entry.key.trim()) {
          envObj[entry.key.trim()] = entry.value;
        }
      }

      await api.updateMcpServerSettings(server.id, { default_policy: defaultPolicy, env: envObj });
      await loadSettings();
      onRefresh();
    });
  }

  // Track initial values to detect actual changes
  const [initialEnvValues, setInitialEnvValues] = useState<Record<string, string>>({});

  // Detect changes
  const envChanged = (() => {
    if (!settings) return false;
    const currentKeys = new Set(envEntries.map((e) => e.key));
    if (currentKeys.size !== initialEnvKeys.size) return true;
    for (const key of initialEnvKeys) {
      if (!currentKeys.has(key)) return true;
    }
    return envEntries.some((e) => e.value !== (initialEnvValues[e.key] ?? ''));
  })();

  const hasChanges = (settings && defaultPolicy !== settings.default_policy) || envChanged;

  const displayError = saveAction.error || loadAction.error;

  return (
    <div className="p-4 space-y-4">
      {displayError && <AlertCard>{displayError}</AlertCard>}

      {/* Server Configuration */}
      <section>
        <h3 className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary mb-2">
          {t('settings_tab.server_config')}
        </h3>
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <label className="text-[10px] font-mono text-content-tertiary w-20">{t('settings_tab.command')}</label>
            <span className="text-xs font-mono text-content-secondary bg-glass rounded px-2 py-1 flex-1">
              {settings?.command ?? server.command ?? '—'}
            </span>
          </div>
          <div className="flex items-center gap-2">
            <label className="text-[10px] font-mono text-content-tertiary w-20">{t('settings_tab.args')}</label>
            <span className="text-xs font-mono text-content-secondary bg-glass rounded px-2 py-1 flex-1 truncate">
              {(settings?.args ?? server.args ?? []).join(' ') || t('settings_tab.none')}
            </span>
          </div>
          <div className="flex items-center gap-2">
            <label className="text-[10px] font-mono text-content-tertiary w-20">{t('settings_tab.transport')}</label>
            <span className="text-xs font-mono text-content-secondary">stdio</span>
          </div>
        </div>
      </section>

      {/* Environment Variables */}
      <section>
        <h3 className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary mb-2">
          {t('settings_tab.env_vars')}
        </h3>
        <EnvVariableEditor
          entries={envEntries}
          onChange={setEnvEntries}
          placeholderKey={t('settings_tab.placeholder_key')}
          placeholderValue={t('settings_tab.placeholder_value')}
          removeLabel={t('settings_tab.remove')}
          addLabel={t('settings_tab.add')}
          emptyHint={t('settings_tab.no_env_hint')}
        />
      </section>

      {/* Default Policy */}
      <section>
        <h3 className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary mb-2">
          {t('settings_tab.default_policy')}
        </h3>
        <select
          value={defaultPolicy}
          onChange={(e) => setDefaultPolicy(e.target.value as DefaultPolicy)}
          className="text-xs font-mono bg-glass border border-edge rounded px-2 py-1 text-content-primary"
        >
          <option value="opt-in">{t('settings_tab.policy_opt_in')}</option>
          <option value="opt-out">{t('settings_tab.policy_opt_out')}</option>
        </select>
        <p className="mt-1 text-[9px] font-mono text-content-tertiary">
          {defaultPolicy === 'opt-in' ? t('settings_tab.policy_opt_in_desc') : t('settings_tab.policy_opt_out_desc')}
        </p>
      </section>

      {/* Manifest */}
      <section>
        <h3 className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary mb-2">
          {t('settings_tab.manifest')}
        </h3>
        <div className="space-y-1">
          <div className="flex gap-2 text-[10px] font-mono">
            <span className="text-content-tertiary w-16">{t('settings_tab.id')}</span>
            <span className="text-content-secondary">{displayServerId(server.id)}</span>
          </div>
          <div className="flex gap-2 text-[10px] font-mono">
            <span className="text-content-tertiary w-16">{t('settings_tab.tools')}</span>
            <span className="text-content-secondary">{server.tools.join(', ') || t('settings_tab.none')}</span>
          </div>
          {settings?.description && (
            <div className="flex gap-2 text-[10px] font-mono">
              <span className="text-content-tertiary w-16">{t('settings_tab.desc')}</span>
              <span className="text-content-secondary">{settings.description}</span>
            </div>
          )}
        </div>
      </section>

      {/* Actions */}
      <div className="flex gap-2 pt-2 border-t border-edge">
        <button
          onClick={handleSave}
          disabled={saveAction.isLoading || !hasChanges}
          aria-label={t('settings_tab.save_changes')}
          className="flex items-center gap-1 px-3 py-1.5 text-[10px] font-mono rounded bg-brand/10 hover:bg-brand/20 text-brand disabled:opacity-40 disabled:cursor-not-allowed transition-colors border border-brand/20"
        >
          <Save size={10} /> {saveAction.isLoading ? t('settings_tab.saving') : t('settings_tab.save_changes')}
        </button>
        <button
          onClick={loadSettings}
          aria-label={t('settings_tab.reset')}
          className="flex items-center gap-1 px-3 py-1.5 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-tertiary transition-colors border border-edge"
        >
          <RotateCcw size={10} /> {t('settings_tab.reset')}
        </button>
      </div>
    </div>
  );
}
