import React, { lazy, Suspense, useCallback, useEffect, useState } from 'react';
import ReactDOM from 'react-dom/client';
import { useTranslation } from 'react-i18next';
import { Route, BrowserRouter as Router, Routes } from 'react-router-dom';
import { AppLayout } from './components/AppLayout';
import { ErrorBoundary } from './components/ErrorBoundary';
import { SetupWizard } from './components/SetupWizard';
import { ThemeProvider } from './components/ThemeProvider';
import { AgentProvider } from './contexts/AgentContext';
import { ApiKeyProvider } from './contexts/ApiKeyContext';
import { ConnectionProvider, useConnection } from './contexts/ConnectionContext';
import { UserIdentityProvider } from './contexts/UserIdentityContext';
import { checkForUpdates, isTauri } from './lib/tauri';
import { api } from './services/api';

import './i18n';
import { loadExternalLanguages } from './i18n';
import './compiled-tailwind.css';

const UPDATE_CHECK_DELAY_MS = 3000;

declare const __APP_VERSION__: string;

const MemoryCore = lazy(() => import('./components/MemoryCore').then((m) => ({ default: m.MemoryCore })));
const McpServersPage = lazy(() => import('./pages/McpServersPage').then((m) => ({ default: m.McpServersPage })));
const CronJobs = lazy(() => import('./components/CronJobs').then((m) => ({ default: m.CronJobs })));
const VrmViewerPage = lazy(() => import('./vrm/VrmViewerPage').then((m) => ({ default: m.VrmViewerPage })));
function App() {
  const [setupDone, setSetupDone] = useState(() => localStorage.getItem('cloto-setup-completed') === '1');

  const [bannerDismissed, setBannerDismissed] = useState(
    () => sessionStorage.getItem('cloto-browser-banner-dismissed') === '1',
  );

  const dismissBanner = () => {
    sessionStorage.setItem('cloto-browser-banner-dismissed', '1');
    setBannerDismissed(true);
  };

  const { connected } = useConnection();
  const { t } = useTranslation();

  // Re-trigger setup wizard if backend reports setup incomplete (e.g. version upgrade)
  useEffect(() => {
    if (!connected || !setupDone) return;
    api
      .getSetupStatus()
      .then((status) => {
        if (!status.setup_complete && !status.setup_in_progress) {
          localStorage.removeItem('cloto-setup-completed');
          setSetupDone(false);
        }
      })
      .catch(() => {
        /* ignore — backend not ready yet */
      });
  }, [connected, setupDone]);

  // Auto-update check on startup (Tauri only, user-configurable)
  useEffect(() => {
    if (!connected || !isTauri) return;
    if (localStorage.getItem('cloto-auto-update') === 'off') return;

    const timer = setTimeout(async () => {
      try {
        const info = await checkForUpdates();
        if (info.available) {
          window.dispatchEvent(new CustomEvent('cloto-update-available', { detail: { version: info.latestVersion } }));
        }
      } catch {
        // Silent fail — network unavailable, rate limited, etc.
      }
    }, UPDATE_CHECK_DELAY_MS);
    return () => clearTimeout(timer);
  }, [connected]);

  // VRM viewer window bypasses connection gate (it loads VRM directly from API)
  const isVrmRoute = window.location.pathname.startsWith('/vrm-viewer/');

  // Rotating flavour text for loading screen
  const BOOT_LINES = t('boot.lines', { returnObjects: true }) as string[];

  const pickRandom = useCallback(
    (prev: number) => {
      let next: number;
      do {
        next = Math.floor(Math.random() * BOOT_LINES.length);
      } while (next === prev);
      return next;
    },
    [BOOT_LINES],
  );

  const [lineIdx, setLineIdx] = useState(() => Math.floor(Math.random() * BOOT_LINES.length));

  useEffect(() => {
    if (connected) return;
    const id = setInterval(() => setLineIdx((prev) => pickRandom(prev)), 6000);
    return () => clearInterval(id);
  }, [connected, pickRandom]);

  // Full-screen loading screen while backend is unreachable
  if (!connected && !isVrmRoute) {
    return (
      <div className="min-h-screen bg-surface-base flex flex-col items-center justify-center select-none">
        <h1 className="text-4xl font-black tracking-[0.2em] text-content-primary">{t('boot.title')}</h1>
        <div className="mt-6 h-6 flex items-center justify-center">
          <p
            key={lineIdx}
            className="text-[11px] font-mono text-brand/70 uppercase tracking-[0.15em] animate-boot-line"
          >
            {BOOT_LINES[lineIdx]}
          </p>
        </div>
      </div>
    );
  }

  // First-run setup wizard (skip for VRM viewer window)
  if (!setupDone && !isVrmRoute) {
    return (
      <SetupWizard
        onComplete={() => {
          localStorage.setItem('cloto-setup-completed', '1');
          setSetupDone(true);
        }}
      />
    );
  }

  return (
    <AgentProvider>
      <Router>
        {!isTauri && !bannerDismissed && (
          <div className="bg-amber-900/80 border-b border-amber-700 px-4 py-2 flex items-center justify-between font-mono text-xs text-amber-200">
            <span>
              {t('browser_banner', {
                defaultValue: 'Browser access is deprecated. Please use the Cloto desktop app for the full experience.',
              })}
            </span>
            <button onClick={dismissBanner} className="ml-4 text-amber-400 hover:text-white" aria-label="Dismiss">
              ✕
            </button>
          </div>
        )}
        <Routes>
          <Route
            path="/vrm-viewer/:agentId"
            element={
              <Suspense fallback={null}>
                <VrmViewerPage />
              </Suspense>
            }
          />
          <Route element={<AppLayout />}>
            <Route path="/" element={null} />
            <Route path="/dashboard" element={<MemoryCore />} />
            <Route path="/mcp-servers" element={<McpServersPage />} />
            <Route path="/cron" element={<CronJobs />} />
          </Route>
        </Routes>
      </Router>
    </AgentProvider>
  );
}

async function bootstrap() {
  await loadExternalLanguages();
  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <ErrorBoundary>
        <ThemeProvider>
          <ApiKeyProvider>
            <UserIdentityProvider>
              <ConnectionProvider>
                <App />
              </ConnectionProvider>
            </UserIdentityProvider>
          </ApiKeyProvider>
        </ThemeProvider>
      </ErrorBoundary>
    </React.StrictMode>,
  );
}

bootstrap();
