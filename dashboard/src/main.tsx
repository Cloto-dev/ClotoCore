import React, { Suspense, lazy, useState, useEffect } from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter as Router, Routes, Route } from 'react-router-dom'
import { Home } from './pages/Home'
import { ErrorBoundary } from './components/ErrorBoundary'
import { ThemeProvider } from './components/ThemeProvider'
import { ApiKeyProvider } from './contexts/ApiKeyContext'
import { ConnectionProvider } from './contexts/ConnectionContext'
import { CustomCursor } from './components/CustomCursor'
import './compiled-tailwind.css'

const StatusCore = lazy(() => import('./components/StatusCore').then(m => ({ default: m.StatusCore })));
const MemoryCore = lazy(() => import('./components/MemoryCore').then(m => ({ default: m.MemoryCore })));
const McpServersPage = lazy(() => import('./pages/McpServersPage').then(m => ({ default: m.McpServersPage })));

function App() {
  const [cursorEnabled, setCursorEnabled] = useState(() => localStorage.getItem('cloto-cursor') !== 'off');

  useEffect(() => {
    const handler = () => setCursorEnabled(localStorage.getItem('cloto-cursor') !== 'off');
    window.addEventListener('cloto-cursor-toggle', handler);
    return () => window.removeEventListener('cloto-cursor-toggle', handler);
  }, []);

  return (
    <Router>
      <Suspense fallback={<div className="min-h-screen bg-surface-base flex items-center justify-center font-mono text-xs text-content-tertiary">LOADING CLOTO...</div>}>
        <Routes>
          <Route path="/" element={<Home />} />
          <Route path="/status" element={<StatusCore />} />
          <Route path="/dashboard" element={<MemoryCore />} />
          <Route path="/mcp-servers" element={<McpServersPage />} />
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
