import { act, render, screen } from '@testing-library/react';
import { I18nextProvider, useTranslation } from 'react-i18next';
import { describe, expect, it } from 'vitest';
import i18n from '../i18n';
import en_common from '../locales/en/common.json';

// Render-level smoke for the react-i18next 17 + i18next 26 bump. Unlike the node
// core check, this mounts a real component through @testing-library/react (jsdom)
// and asserts the React render path + reactive re-render on changeLanguage — the
// exact mechanism the app uses for language switching (external packs are
// registered via addResourceBundle, then activated via changeLanguage). Runs in
// the existing vitest/jsdom suite (CI "Dashboard" job, ubuntu/Linux).

function Probe() {
  const { t } = useTranslation('common');
  return <div data-testid="v">{t('save')}</div>;
}

describe('react-i18next 17: render + language switch (jsdom)', () => {
  it('renders the EN translation, then re-renders when the active language changes', async () => {
    await act(async () => {
      await i18n.changeLanguage('en');
    });

    render(
      <I18nextProvider i18n={i18n}>
        <Probe />
      </I18nextProvider>,
    );

    // 1) React render path resolves the EN resource (not the bare key)
    expect(screen.getByTestId('v').textContent).toBe(en_common.save);
    expect(en_common.save).not.toBe('save');

    // 2) external-pack path: addResourceBundle a new language, then changeLanguage,
    //    and assert the mounted component re-renders with the switched text.
    const xx = JSON.parse(JSON.stringify(en_common));
    xx.save = `${en_common.save} [XX]`;
    i18n.addResourceBundle('xx', 'common', xx, true, true);
    await act(async () => {
      await i18n.changeLanguage('xx');
    });
    expect(screen.getByTestId('v').textContent).toBe(`${en_common.save} [XX]`);

    // restore shared instance state for other tests
    await act(async () => {
      await i18n.changeLanguage('en');
    });
    i18n.removeResourceBundle('xx', 'common');
    expect(screen.getByTestId('v').textContent).toBe(en_common.save);
  });
});
