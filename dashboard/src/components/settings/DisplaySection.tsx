import { useTranslation } from 'react-i18next';
import { useLocalStorage } from '../../hooks/useStorage';
import { SectionCard, Toggle } from './common';

export function DisplaySection() {
  const { t } = useTranslation('settings');
  const [cursorRaw, setCursorRaw] = useLocalStorage('cloto-cursor', 'on');
  const cursorEnabled = cursorRaw !== 'off';

  const handleCursorToggle = () => {
    setCursorRaw(cursorEnabled ? 'off' : 'on');
    window.dispatchEvent(new Event('cloto-cursor-toggle'));
  };

  return (
    <SectionCard title={t('display.cursor_title')}>
      <div className="space-y-4">
        <Toggle enabled={cursorEnabled} onToggle={handleCursorToggle} label={t('display.cursor_label')} />
        <p className="text-xs text-content-tertiary">{t('display.cursor_desc')}</p>
      </div>
    </SectionCard>
  );
}
