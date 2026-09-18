import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './App';
import { ErrorBoundary } from './components/ErrorBoundary';
import { ShutdownOverlay } from './components/ShutdownOverlay';
import { ThemeProvider } from './components/ThemeProvider';
import { ApiKeyProvider } from './contexts/ApiKeyContext';
import { ConnectionProvider } from './contexts/ConnectionContext';
import { UserIdentityProvider } from './contexts/UserIdentityContext';
import { restoreBrowserSession } from './services/session';

import './i18n';
import { applyStoredTheme } from './hooks/useTheme';
import { loadExternalLanguages } from './i18n';
import { loadThemes } from './themes/load';
// Bundled typefaces (docs/DESIGN_PHILOSOPHY.md §4.3), split by unicode-range so a
// page loads only the subsets it draws. License: public/fonts/LICENSE-IBM-Plex.txt
import '@fontsource/ibm-plex-sans-jp/400.css';
import '@fontsource/ibm-plex-sans-jp/500.css';
import '@fontsource/ibm-plex-sans-jp/600.css';
import '@fontsource/ibm-plex-mono/400.css';
import '@fontsource/ibm-plex-mono/500.css';
import './compiled-tailwind.css';

async function bootstrap() {
  // Before the first component mounts, so that the things which cannot carry a
  // header — the event stream, avatars, the VRM model — have a credential by the
  // time they are rendered. Deliberately not awaited (see the function's docs).
  restoreBrowserSession();
  await loadExternalLanguages();
  // Before React: the theme decides the accent's surface and whether the accent
  // is an agent's at all, and the first agent colour is computed during render.
  await loadThemes();
  applyStoredTheme();
  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <ErrorBoundary>
        <ThemeProvider>
          <ApiKeyProvider>
            <UserIdentityProvider>
              <ConnectionProvider>
                <App />
                {/* Above the connection gate: must survive the kernel going
                    away mid-shutdown. */}
                <ShutdownOverlay />
              </ConnectionProvider>
            </UserIdentityProvider>
          </ApiKeyProvider>
        </ThemeProvider>
      </ErrorBoundary>
    </React.StrictMode>,
  );
}

bootstrap();
