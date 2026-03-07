import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Sun, Moon, Monitor, Users, Server, Clock, Brain, Settings } from 'lucide-react';
import { useTheme } from '../hooks/useTheme';
import { useUserIdentity } from '../contexts/UserIdentityContext';
import { getCustomLanguages } from '../i18n';

const BUILTIN_LANGUAGES = [
  { code: 'en', label: 'English' },
  { code: 'ja', label: '日本語' },
];

const TOTAL_STEPS = 5;

interface Props {
  onComplete: () => void;
}

export function SetupWizard({ onComplete }: Props) {
  const [step, setStep] = useState(0);
  const { t, i18n } = useTranslation('wizard');
  const { preference, setPreference } = useTheme();
  const { identity, setIdentity } = useUserIdentity();
  const [customLangs, setCustomLangs] = useState<{ code: string; label: string }[]>([]);
  const [displayName, setDisplayName] = useState(identity.name === 'User' ? '' : identity.name);

  useEffect(() => {
    getCustomLanguages().then(setCustomLangs);
  }, []);

  const builtinCodes = new Set(BUILTIN_LANGUAGES.map(l => l.code));
  const allLanguages = [
    ...BUILTIN_LANGUAGES,
    ...customLangs.filter(l => !builtinCodes.has(l.code)),
  ];

  const next = () => setStep(s => Math.min(s + 1, TOTAL_STEPS - 1));
  const back = () => setStep(s => Math.max(s - 1, 0));

  const handleFinish = () => {
    if (displayName.trim()) {
      setIdentity(identity.id, displayName.trim());
    }
    onComplete();
  };

  const handleNameBlur = () => {
    if (displayName.trim()) {
      setIdentity(identity.id, displayName.trim());
    }
  };

  const themes = [
    { value: 'light' as const, icon: Sun, label: t('theme_light') },
    { value: 'dark' as const, icon: Moon, label: t('theme_dark') },
    { value: 'system' as const, icon: Monitor, label: t('theme_system') },
  ];

  const guideItems = [
    { icon: Users, label: 'Agent', desc: t('guide_agents') },
    { icon: Server, label: 'MCP', desc: t('guide_mcp') },
    { icon: Clock, label: 'Cron', desc: t('guide_cron') },
    { icon: Brain, label: 'Memory', desc: t('guide_memory') },
    { icon: Settings, label: 'Settings', desc: t('guide_settings') },
  ];

  return (
    <div className="fixed inset-0 z-50 bg-surface-base flex items-center justify-center">
      <div className="bg-surface-primary border border-edge rounded-2xl shadow-2xl w-full max-w-lg mx-4 flex flex-col">
        {/* Content */}
        <div className="p-8 min-h-[340px] flex flex-col items-center justify-center">
          {step === 0 && (
            <div className="text-center space-y-6">
              <h1 className="text-3xl font-black tracking-[0.15em] text-content-primary">
                CLOTO SYSTEM
              </h1>
              <p className="text-sm text-content-secondary max-w-sm">
                {t('welcome_desc')}
              </p>
              <button
                onClick={next}
                className="px-8 py-3 bg-brand text-white rounded-xl text-sm font-bold hover:opacity-90 transition-opacity"
              >
                {t('get_started')}
              </button>
            </div>
          )}

          {step === 1 && (
            <div className="text-center space-y-6 w-full max-w-xs">
              <h2 className="text-xl font-bold text-content-primary">
                {t('select_language')}
              </h2>
              <select
                value={i18n.language.split('-')[0]}
                onChange={e => i18n.changeLanguage(e.target.value)}
                className="w-full px-4 py-3 bg-surface-secondary border border-edge rounded-xl text-sm text-content-primary focus:border-brand focus:outline-none transition-colors"
              >
                {allLanguages.map(lang => (
                  <option key={lang.code} value={lang.code}>
                    {lang.label}
                  </option>
                ))}
              </select>
            </div>
          )}

          {step === 2 && (
            <div className="text-center space-y-6">
              <h2 className="text-xl font-bold text-content-primary">
                {t('select_theme')}
              </h2>
              <div className="flex gap-3">
                {themes.map(({ value, icon: Icon, label }) => (
                  <button
                    key={value}
                    onClick={() => setPreference(value)}
                    className={`flex items-center gap-2 px-5 py-3 rounded-xl text-sm font-bold transition-all ${
                      preference === value
                        ? 'bg-brand text-white shadow-md'
                        : 'bg-surface-secondary text-content-secondary hover:text-content-primary border border-edge hover:border-brand'
                    }`}
                  >
                    <Icon size={16} />
                    {label}
                  </button>
                ))}
              </div>
            </div>
          )}

          {step === 3 && (
            <div className="text-center space-y-6 w-full max-w-xs">
              <h2 className="text-xl font-bold text-content-primary">
                {t('enter_name')}
              </h2>
              <input
                type="text"
                value={displayName}
                onChange={e => setDisplayName(e.target.value)}
                onBlur={handleNameBlur}
                placeholder={t('name_placeholder')}
                className="w-full px-4 py-3 bg-surface-secondary border border-edge rounded-xl text-sm text-content-primary focus:border-brand focus:outline-none transition-colors text-center"
              />
              <p className="text-[11px] text-content-tertiary">
                {t('name_hint')}
              </p>
            </div>
          )}

          {step === 4 && (
            <div className="space-y-5 w-full">
              <h2 className="text-xl font-bold text-content-primary text-center">
                {t('quick_guide')}
              </h2>
              <div className="space-y-3">
                {guideItems.map(({ icon: Icon, label, desc }) => (
                  <div key={label} className="flex items-start gap-3 px-4 py-3 bg-surface-secondary rounded-xl border border-edge">
                    <Icon size={18} className="text-brand shrink-0 mt-0.5" />
                    <div>
                      <span className="text-xs font-bold text-content-primary">{label}</span>
                      <p className="text-[11px] text-content-secondary mt-0.5">{desc}</p>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Footer: dots + nav buttons */}
        <div className="px-8 pb-6 flex items-center justify-between">
          {/* Back button */}
          <div className="w-20">
            {step > 0 && step < TOTAL_STEPS && (
              <button
                onClick={back}
                className="text-xs font-bold text-content-tertiary hover:text-content-primary transition-colors"
              >
                {t('back')}
              </button>
            )}
          </div>

          {/* Step dots */}
          <div className="flex gap-2">
            {Array.from({ length: TOTAL_STEPS }, (_, i) => (
              <div
                key={i}
                className={`w-2 h-2 rounded-full transition-colors ${
                  i === step ? 'bg-brand' : 'bg-edge'
                }`}
              />
            ))}
          </div>

          {/* Next / Finish button */}
          <div className="w-20 flex justify-end">
            {step === 0 ? (
              <div /> // Welcome has its own CTA
            ) : step < TOTAL_STEPS - 1 ? (
              <button
                onClick={next}
                className="px-4 py-2 bg-brand text-white rounded-lg text-xs font-bold hover:opacity-90 transition-opacity"
              >
                {t('next')}
              </button>
            ) : (
              <button
                onClick={handleFinish}
                className="px-4 py-2 bg-brand text-white rounded-lg text-xs font-bold hover:opacity-90 transition-opacity whitespace-nowrap"
              >
                {t('finish')}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
