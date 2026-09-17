import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { Select, Toggle } from '../common';

const OPTIONS = [
  { value: 'ja', label: '日本語' },
  { value: 'en', label: 'English' },
];

describe('the settings picker', () => {
  it('opens a listbox rather than handing the platform a native select', () => {
    render(<Select label="language" value="ja" options={OPTIONS} onChange={vi.fn()} />);
    const button = screen.getByRole('button', { name: 'language' });
    expect(button.getAttribute('aria-haspopup')).toBe('listbox');
    expect(button.textContent).toContain('日本語');
    expect(screen.queryByRole('listbox')).toBeNull();

    fireEvent.click(button);
    expect(screen.getByRole('listbox')).toBeTruthy();
    const options = screen.getAllByRole('option');
    expect(options.map((o) => o.getAttribute('aria-selected'))).toEqual(['true', 'false']);
  });

  it('moves with the arrows and takes the option Enter is on', () => {
    const onChange = vi.fn();
    render(<Select label="language" value="ja" options={OPTIONS} onChange={onChange} />);
    const button = screen.getByRole('button', { name: 'language' });

    fireEvent.keyDown(button, { key: 'Enter' });
    fireEvent.keyDown(button, { key: 'ArrowDown' });
    fireEvent.keyDown(button, { key: 'Enter' });

    // The value of the option the arrow landed on — not the index, and not the
    // one that was already selected.
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith('en');
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('abandons on Escape, choosing nothing', () => {
    const onChange = vi.fn();
    render(<Select label="language" value="ja" options={OPTIONS} onChange={onChange} />);
    const button = screen.getByRole('button', { name: 'language' });

    fireEvent.keyDown(button, { key: 'Enter' });
    fireEvent.keyDown(button, { key: 'ArrowDown' });
    fireEvent.keyDown(button, { key: 'Escape' });

    expect(screen.queryByRole('listbox')).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it('closes when the click lands somewhere else', () => {
    render(
      <div>
        <Select label="language" value="ja" options={OPTIONS} onChange={vi.fn()} />
        <button type="button">elsewhere</button>
      </div>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'language' }));
    expect(screen.getByRole('listbox')).toBeTruthy();

    fireEvent.pointerDown(screen.getByText('elsewhere'));
    expect(screen.queryByRole('listbox')).toBeNull();
  });
});

function Switchable() {
  const [on, setOn] = useState(false);
  return <Toggle label="inject" checked={on} onChange={() => setOn((v) => !v)} />;
}

describe('the settings switch', () => {
  it('says it is a switch and which way it is set', () => {
    render(<Toggle label="inject" checked={true} onChange={vi.fn()} />);
    const toggle = screen.getByRole('switch', { name: 'inject' });
    expect(toggle.getAttribute('aria-checked')).toBe('true');
    // The track when on is neutral: the workshop has no accent to spend here.
    expect(toggle.className).toBe('tgl on');
  });

  it('flips through onChange, and not on its own', () => {
    render(<Switchable />);
    const toggle = screen.getByRole('switch', { name: 'inject' });
    expect(toggle.getAttribute('aria-checked')).toBe('false');

    fireEvent.click(toggle);
    expect(screen.getByRole('switch', { name: 'inject' }).getAttribute('aria-checked')).toBe('true');

    fireEvent.click(screen.getByRole('switch', { name: 'inject' }));
    expect(screen.getByRole('switch', { name: 'inject' }).getAttribute('aria-checked')).toBe('false');
  });

  it('does not call onChange while disabled', () => {
    const onChange = vi.fn();
    render(<Toggle label="inject" checked={false} onChange={onChange} disabled />);
    fireEvent.click(screen.getByRole('switch', { name: 'inject' }));
    expect(onChange).not.toHaveBeenCalled();
  });
});
