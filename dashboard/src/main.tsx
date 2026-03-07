import React, { lazy, useState, useEffect, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import ReactDOM from 'react-dom/client'
import { BrowserRouter as Router, Routes, Route } from 'react-router-dom'
import { ErrorBoundary } from './components/ErrorBoundary'
import { ThemeProvider } from './components/ThemeProvider'
import { ApiKeyProvider } from './contexts/ApiKeyContext'
import { UserIdentityProvider } from './contexts/UserIdentityContext'
import { ConnectionProvider, useConnection } from './contexts/ConnectionContext'
import { AgentProvider } from './contexts/AgentContext'
import { AppLayout } from './components/AppLayout'
import { AgentPage } from './pages/AgentPage'
import { CustomCursor } from './components/CustomCursor'
import { isTauri } from './lib/tauri'
import './i18n';
import { loadExternalLanguages } from './i18n';
import './compiled-tailwind.css'

declare const __APP_VERSION__: string;

const MemoryCore = lazy(() => import('./components/MemoryCore').then(m => ({ default: m.MemoryCore })));
const McpServersPage = lazy(() => import('./pages/McpServersPage').then(m => ({ default: m.McpServersPage })));
const CronJobs = lazy(() => import('./components/CronJobs').then(m => ({ default: m.CronJobs })));

function App() {
  const [cursorEnabled, setCursorEnabled] = useState(() => localStorage.getItem('cloto-cursor') !== 'off');

  useEffect(() => {
    const handler = () => setCursorEnabled(localStorage.getItem('cloto-cursor') !== 'off');
    window.addEventListener('cloto-cursor-toggle', handler);
    return () => window.removeEventListener('cloto-cursor-toggle', handler);
  }, []);

  const [bannerDismissed, setBannerDismissed] = useState(() => sessionStorage.getItem('cloto-browser-banner-dismissed') === '1');

  const dismissBanner = () => {
    sessionStorage.setItem('cloto-browser-banner-dismissed', '1');
    setBannerDismissed(true);
  };

  const { connected } = useConnection();
  const { t } = useTranslation();

  // Rotating flavour text for loading screen
  const BOOT_LINES = t('boot.lines', { returnObjects: true }) as string[];

  const pickRandom = useCallback((prev: number) => {
    let next: number;
    do { next = Math.floor(Math.random() * BOOT_LINES.length); } while (next === prev);
    return next;
  }, []);

  const [lineIdx, setLineIdx] = useState(() => Math.floor(Math.random() * BOOT_LINES.length));

  useEffect(() => {
    if (connected) return;
    const id = setInterval(() => setLineIdx(prev => pickRandom(prev)), 6000);
    return () => clearInterval(id);
  }, [connected, pickRandom]);

  // Full-screen loading screen while backend is unreachable
  if (!connected) {
    return (
      <div className="min-h-screen bg-surface-base flex flex-col items-center justify-center select-none">
        <h1 className="text-4xl font-black tracking-[0.2em] text-content-primary">
          {t('boot.title')}
        </h1>
        <div className="mt-6 h-6 flex items-center justify-center">
          <p key={lineIdx} className="text-[11px] font-mono text-brand/70 uppercase tracking-[0.15em] animate-boot-line">
            {BOOT_LINES[lineIdx]}
          </p>
        </div>
      </div>
    );
  }

  return (
    <AgentProvider>
      <Router>
        {!isTauri && !bannerDismissed && (
          <div className="bg-amber-900/80 border-b border-amber-700 px-4 py-2 flex items-center justify-between font-mono text-xs text-amber-200">
            <span>
              {t('browser_banner', { defaultValue: 'Browser access is deprecated. Please use the Cloto desktop app for the full experience.' })}
            </span>
            <button onClick={dismissBanner} className="ml-4 text-amber-400 hover:text-white" aria-label="Dismiss">
              ✕
            </button>
          </div>
        )}
        <Routes>
          <Route element={<AppLayout />}>
            <Route path="/" element={<AgentPage />} />
            <Route path="/dashboard" element={<MemoryCore />} />
            <Route path="/mcp-servers" element={<McpServersPage />} />
            <Route path="/cron" element={<CronJobs />} />
          </Route>
        </Routes>
        {cursorEnabled && <CustomCursor />}
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
