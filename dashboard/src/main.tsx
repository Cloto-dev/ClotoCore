import React, { Suspense, lazy } from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter as Router, Routes, Route } from 'react-router-dom'
import { Home } from './pages/Home'
import { ErrorBoundary } from './components/ErrorBoundary'
import { ThemeProvider } from './components/ThemeProvider'
import { ApiKeyProvider } from './contexts/ApiKeyContext'
import { CustomCursor } from './components/CustomCursor'
import './compiled-tailwind.css'

const StatusCore = lazy(() => import('./components/StatusCore').then(m => ({ default: m.StatusCore })));
const MemoryCore = lazy(() => import('./components/MemoryCore').then(m => ({ default: m.MemoryCore })));
const EvolutionCore = lazy(() => import('./components/EvolutionCore').then(m => ({ default: m.EvolutionCore })));

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <ThemeProvider>
        <ApiKeyProvider>
        <Router>
          <Suspense fallback={<div className="min-h-screen bg-surface-base flex items-center justify-center font-mono text-xs text-content-tertiary">LOADING EXIV...</div>}>
            <Routes>
              <Route path="/" element={<Home />} />
              <Route path="/status" element={<StatusCore />} />
              <Route path="/dashboard" element={<MemoryCore />} />
              <Route path="/evolution" element={<EvolutionCore />} />
            </Routes>
          </Suspense>
          <CustomCursor />
        </Router>
        </ApiKeyProvider>
      </ThemeProvider>
    </ErrorBoundary>
  </React.StrictMode>,
)
