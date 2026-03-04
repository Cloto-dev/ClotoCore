import React, { Suspense, lazy, useState, useEffect, useCallback } from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter as Router, Routes, Route } from 'react-router-dom'
import { Home } from './pages/Home'
import { ErrorBoundary } from './components/ErrorBoundary'
import { ThemeProvider } from './components/ThemeProvider'
import { ApiKeyProvider } from './contexts/ApiKeyContext'
import { ConnectionProvider, useConnection } from './contexts/ConnectionContext'
import { CustomCursor } from './components/CustomCursor'
import { isTauri } from './lib/tauri'
import './compiled-tailwind.css'

declare const __APP_VERSION__: string;

const StatusCore = lazy(() => import('./components/StatusCore').then(m => ({ default: m.StatusCore })));
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

  // Rotating flavour text for loading screen
  const BOOT_LINES = [
    'Initializing neural pathways...',
    'Calibrating cognitive matrices...',
    'Establishing secure channels...',
    'Loading agent consciousness...',
    'Synchronizing memory cores...',
    'Bootstrapping reasoning engines...',
    'Activating local intelligence...',
    'Verifying system integrity...',
  ];

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
        <h1 className="text-4xl font-black tracking-[0.2em] text-content-primary animate-pulse">
          CLOTO SYSTEM
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
    <Router>
      {!isTauri && !bannerDismissed && (
        <div className="bg-amber-900/80 border-b border-amber-700 px-4 py-2 flex items-center justify-between font-mono text-xs text-amber-200">
          <span>
            Browser access is deprecated. Please use the{' '}
            <a href="https://github.com/Cloto-dev/ClotoCore/releases/latest" target="_blank" rel="noopener noreferrer" className="underline text-amber-100 hover:text-white">
              Cloto desktop app
            </a>{' '}
            for the full experience.
          </span>
          <button onClick={dismissBanner} className="ml-4 text-amber-400 hover:text-white" aria-label="Dismiss">
            ✕
          </button>
        </div>
      )}
      <Suspense fallback={<div className="min-h-screen bg-surface-base flex items-center justify-center font-mono text-xs text-content-tertiary">LOADING CLOTO...</div>}>
        <Routes>
          <Route path="/" element={<Home />} />
          <Route path="/status" element={<StatusCore />} />
          <Route path="/dashboard" element={<MemoryCore />} />
          <Route path="/mcp-servers" element={<McpServersPage />} />
          <Route path="/cron" element={<CronJobs />} />
        </Routes>
      </Suspense>
      {cursorEnabled && <CustomCursor />}
    </Router>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <ThemeProvider>
        <ApiKeyProvider>
          <ConnectionProvider>
            <App />
          </ConnectionProvider>
        </ApiKeyProvider>
      </ThemeProvider>
    </ErrorBoundary>
  </React.StrictMode>,
)
