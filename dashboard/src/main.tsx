import React, { Suspense, lazy } from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter as Router, Routes, Route } from 'react-router-dom'
import { Home } from './pages/Home'
import { ErrorBoundary } from './components/ErrorBoundary'
import './compiled-tailwind.css'

const StatusCore = lazy(() => import('./components/StatusCore').then(m => ({ default: m.StatusCore })));
const MemoryCore = lazy(() => import('./components/MemoryCore').then(m => ({ default: m.MemoryCore })));

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <Router>
        <Suspense fallback={<div className="min-h-screen bg-slate-50 flex items-center justify-center font-mono text-xs text-slate-400">LOADING EXIV...</div>}>
          <Routes>
            <Route path="/" element={<Home />} />
            <Route path="/status" element={<StatusCore />} />
            <Route path="/dashboard" element={<MemoryCore />} />
          </Routes>
        </Suspense>
      </Router>
    </ErrorBoundary>
  </React.StrictMode>,
)
