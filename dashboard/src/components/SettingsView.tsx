import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import {
  AboutSection,
  AdvancedSection,
  ConversationsSection,
  GeneralSection,
  HealthSection,
  LogSection,
  SecuritySection,
} from './settings';
import './settings/Settings.css';
import './Workshop.css';

export type Section = 'general' | 'conversations' | 'security' | 'advanced' | 'health' | 'log' | 'about';

const SECTIONS: Section[] = ['general', 'conversations', 'security', 'advanced', 'health', 'log', 'about'];

/** `?section=` names one of the seven; anything else opens the first. */
export function sectionFromQuery(value: string | null): Section {
  return (SECTIONS as string[]).includes(value ?? '') ? (value as Section) : 'general';
}

/**
 * Settings (docs/gui/samples/05-settings.html): the sections on the left, and
 * on the right rows of "what it is, what it is for, and the control".
 *
 * It is a page rather than a dialog. Which section is open is in the URL, so
 * the update notice can arrive at About and the window bar's Back leaves the
 * page in one step — section clicks replace the entry rather than stack on it.
 */
export function SettingsView() {
  const { t } = useTranslation('settings');
  const [searchParams, setSearchParams] = useSearchParams();
  const section = sectionFromQuery(searchParams.get('section'));

  const open = (next: Section) => {
    const params = new URLSearchParams(searchParams);
    params.set('section', next);
    setSearchParams(params, { replace: true });
  };

  return (
    <div className="ws">
      <div className="ws-head">
        <h1>{t('title')}</h1>
      </div>

      <div className="ws-body set-body">
        <nav className="rail" aria-label={t('title')}>
          {SECTIONS.map((id) => (
            <button
              type="button"
              key={id}
              className={section === id ? 'on' : undefined}
              aria-current={section === id ? 'page' : undefined}
              onClick={() => open(id)}
            >
              {t(`sections.${id}`)}
            </button>
          ))}
        </nav>

        <div className="set-pane">
          {section === 'general' && <GeneralSection />}
          {section === 'conversations' && <ConversationsSection />}
          {section === 'security' && <SecuritySection />}
          {section === 'advanced' && <AdvancedSection />}
          {section === 'health' && <HealthSection />}
          {section === 'log' && <LogSection />}
          {section === 'about' && <AboutSection />}
        </div>
      </div>
    </div>
  );
}
