import { Brain, Clock, ExternalLink, MessageCircle, Server, Settings, Users } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ISSUES_URL, REPOSITORY_URL } from '../constants';

declare const __APP_VERSION__: string;

interface HelpContentProps {
  onAskAgent: () => void;
}

const menuGuide: Array<{
  icon: typeof Users;
  labelKey: string;
  descKey: string;
  label?: string;
}> = [
  { icon: Users, labelKey: 'nav:agent', descKey: 'help.agent_desc' },
  { icon: Server, labelKey: 'nav:mcp', descKey: 'help.mcp_desc' },
  { icon: Clock, labelKey: 'nav:cron', descKey: 'help.cron_desc' },
  { icon: Brain, labelKey: 'nav:memory', descKey: 'help.memory_desc' },
  { icon: Settings, labelKey: 'nav:settings', descKey: 'help.settings_desc' },
];

export function HelpContent({ onAskAgent }: HelpContentProps) {
  const { t } = useTranslation('common');
  const { t: tNav } = useTranslation('nav');

  return (
    <>
      {/* Menu Guide */}
      <div className="px-5 py-4 space-y-3">
        <p className="text-xs font-mono text-content-tertiary mb-3">{t('help.navigation')}</p>
        {menuGuide.map(({ icon: Icon, labelKey, label, descKey }) => (
          <div key={descKey} className="flex items-start gap-3">
            <Icon size={14} className="text-agent mt-0.5 shrink-0" />
            <div>
              <span className="text-xs font-mono font-bold text-content-primary">
                {label ??
                  (labelKey ? (labelKey.includes(':') ? tNav(labelKey.split(':')[1]) : t(labelKey)) : '').toUpperCase()}
              </span>
              <span className="text-xs text-content-secondary ml-2">{t(descKey)}</span>
            </div>
          </div>
        ))}
      </div>

      {/* Divider */}
      <div className="border-t border-edge mx-5" />

      {/* Ask Agent */}
      <div className="px-5 py-4">
        <button
          onClick={onAskAgent}
          className="w-full flex items-center justify-center gap-2 px-4 py-2.5 rounded-lg bg-agent/10 hover:bg-agent/20 border border-agent/30 text-agent text-xs font-mono font-bold transition-colors"
        >
          <MessageCircle size={14} />
          {t('help.ask_assistant')}
        </button>
        <p className="text-xs text-content-tertiary text-center mt-2 font-mono">{t('help.ask_hint')}</p>
      </div>

      {/* Footer */}
      <div className="border-t border-edge px-5 py-3 flex items-center justify-between">
        <div className="flex items-center gap-3 text-xs font-mono text-content-tertiary">
          <a
            href={REPOSITORY_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="hover:text-content-secondary transition-colors flex items-center gap-1"
          >
            {t('help.docs')} <ExternalLink size={8} />
          </a>
          <a
            href={ISSUES_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="hover:text-content-secondary transition-colors flex items-center gap-1"
          >
            {t('help.github')} <ExternalLink size={8} />
          </a>
        </div>
        <span className="text-xs font-mono text-content-tertiary">v{__APP_VERSION__}</span>
      </div>
    </>
  );
}
