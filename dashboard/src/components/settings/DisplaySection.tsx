import { SectionCard, Toggle } from './common';
import { useLocalStorage } from '../../hooks/useStorage';

export function DisplaySection() {
  const [cursorRaw, setCursorRaw] = useLocalStorage('cloto-cursor', 'on');
  const cursorEnabled = cursorRaw !== 'off';

  const handleCursorToggle = () => {
    setCursorRaw(cursorEnabled ? 'off' : 'on');
    window.dispatchEvent(new Event('cloto-cursor-toggle'));
  };

  return (
    <SectionCard title="Cursor">
      <div className="space-y-4">
        <Toggle enabled={cursorEnabled} onToggle={handleCursorToggle} label="Custom animated cursor" />
        <p className="text-xs text-content-tertiary">Replaces the native cursor with an animated trail effect using canvas rendering.</p>
      </div>
    </SectionCard>
  );
}
