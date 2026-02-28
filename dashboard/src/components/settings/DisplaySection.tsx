import { useState } from 'react';
import { SectionCard, Toggle } from './common';

export function DisplaySection() {
  const [cursorEnabled, setCursorEnabled] = useState(() => localStorage.getItem('cloto-cursor') !== 'off');

  const handleCursorToggle = () => {
    const next = !cursorEnabled;
    setCursorEnabled(next);
    localStorage.setItem('cloto-cursor', next ? 'on' : 'off');
    window.dispatchEvent(new Event('cloto-cursor-toggle'));
  };

  return (
    <SectionCard title="Cursor">
      <div className="space-y-4">
        <Toggle enabled={cursorEnabled} onToggle={handleCursorToggle} label="Custom animated cursor" />
        <p className="text-[10px] text-content-muted">Replaces the native cursor with an animated trail effect using canvas rendering.</p>
      </div>
    </SectionCard>
  );
}
