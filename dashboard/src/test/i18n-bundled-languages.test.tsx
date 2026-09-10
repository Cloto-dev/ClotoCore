import { act, render, screen } from '@testing-library/react';
import { I18nextProvider, useTranslation } from 'react-i18next';
import { describe, expect, it } from 'vitest';
import i18n, { BUILTIN_LANGUAGES, NAMESPACES } from '../i18n';
import en_common from '../locales/en/common.json';
import ja_pack from '../locales/packs/ja.json';

// The picker offers BUILTIN_LANGUAGES unconditionally. Until 2026-09-10 it named
// Japanese while the Japanese resources arrived only through a Tauri filesystem
// call, so over HTTP the browser offered a language it could not render:
// changeLanguage('ja') succeeded, fallbackLng resolved every key back to English,
// and nothing anywhere reported a problem. These assert the property that failed
// — an offered language has resources — rather than the one language that failed,
// so the next language added to the picker is covered too.

describe('bundled languages are actually bundled', () => {
  const namespaces = NAMESPACES;

  it('registers every namespace for every language the picker offers', () => {
    expect(namespaces.length).toBeGreaterThan(0);
    expect(BUILTIN_LANGUAGES.length).toBeGreaterThan(1);

    for (const { code } of BUILTIN_LANGUAGES) {
      for (const ns of namespaces) {
        expect(
          i18n.hasResourceBundle(code, ns),
          `${code}/${ns} is offered in the picker but has no bundled resources`,
        ).toBe(true);
      }
    }
  });

  it('renders the Japanese string, not the English fallback', async () => {
    function Probe() {
      const { t } = useTranslation('common');
      return <div data-testid="v">{t('save')}</div>;
    }

    await act(async () => {
      await i18n.changeLanguage('ja');
    });
    render(
      <I18nextProvider i18n={i18n}>
        <Probe />
      </I18nextProvider>,
    );

    // Guard the guard: if the two locales ever share this string, the assertion
    // below would pass on a pure-English fallback and prove nothing.
    expect(ja_pack.common.save).not.toBe(en_common.save);
    expect(screen.getByTestId('v').textContent).toBe(ja_pack.common.save);

    await act(async () => {
      await i18n.changeLanguage('en');
    });
  });
});
