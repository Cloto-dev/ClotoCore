import type { ReactNode } from 'react';
import { Select, type SelectOption } from '../ui/Select';

/**
 * The parts every settings section is drawn from
 * (docs/gui/samples/05-settings.html): a heading, and under it rows that each
 * say what they are and what they are for, with one control on the right.
 *
 * The picker is the workshop's own (`ui/Select.tsx`, ported from
 * 08-agent-settings.html) — a native `<select>` cannot be drawn, and there is
 * no reason for this page to have a second one.
 */
export type { SelectOption };
export { Select };

/** A heading and the rows under it. */
export function SettingsGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <>
      <h2>{title}</h2>
      {children}
    </>
  );
}

/**
 * One row. `desc` is the sentence that says what the setting is for — the
 * mock puts one under nearly every label, because a name alone does not tell
 * anyone whether to change it.
 */
export function SettingsRow({
  label,
  desc,
  align = 'center',
  children,
}: {
  label: ReactNode;
  desc?: ReactNode;
  /** `tall` for a control taller than a line, `stacked` for one that needs the width. */
  align?: 'center' | 'tall' | 'stacked';
  children?: ReactNode;
}) {
  const cls = align === 'center' ? 'srow' : `srow ${align}`;
  return (
    <div className={cls}>
      <div className="k">
        <div className="lbl">{label}</div>
        {desc && <div className="d">{desc}</div>}
      </div>
      {children && <div className="v">{children}</div>}
    </div>
  );
}

/** A choice among a few, all of them worth showing at once. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  /** The accessible name of the group — the buttons carry only their own. */
  label: string;
}) {
  return (
    <fieldset className="seg" aria-label={label}>
      {options.map((o) => (
        <button
          type="button"
          key={o.value}
          className={o.value === value ? 'on' : undefined}
          aria-pressed={o.value === value}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </fieldset>
  );
}

/**
 * On or off. The track takes the secondary text colour when on, not the
 * agent's — the workshop has no accent (docs/DESIGN_PHILOSOPHY.md §4.2).
 */
export function Toggle({
  checked,
  onChange,
  label,
  disabled = false,
}: {
  checked: boolean;
  onChange: () => void;
  /** The accessible name: the row's label, which this control has no copy of. */
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={checked ? 'tgl on' : 'tgl'}
      onClick={onChange}
    />
  );
}
