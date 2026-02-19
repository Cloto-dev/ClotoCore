import React, { useState, useEffect } from 'react';
import { Shield, Lock, Unlock, AlertTriangle, X, Check, ShieldAlert } from 'lucide-react';
import { api } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';

interface PermissionRequest {
  request_id: string;
  plugin_id: string;
  permission_type: string;
  target_resource?: string;
  justification: string;
  status: string;
  created_at: string;
}

export function SecurityGuard() {
  const { apiKey } = useApiKey();
  const [requests, setRequests] = useState<PermissionRequest[]>([]);
  const [authorizingIds, setAuthorizingIds] = useState<string[]>([]);
  const [grantedIds, setGrantedIds] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  // M-23: Poll for pending permission requests with AbortController
  useEffect(() => {
    let abortController: AbortController | null = null;

    const fetchPending = async () => {
      abortController?.abort();
      abortController = new AbortController();
      try {
        const pending = await api.getPendingPermissions();
        if (!abortController.signal.aborted) {
          setRequests(pending);
        }
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return;
        console.error("Failed to fetch pending permissions:", err);
      }
    };

    fetchPending();
    const interval = setInterval(fetchPending, 3000);
    return () => {
      clearInterval(interval);
      abortController?.abort();
    };
  }, []);

  const handleGrant = async (req: PermissionRequest) => {
    const reqId = req.request_id;
    setAuthorizingIds(prev => [...prev, reqId]);
    setError(null);

    try {
      await api.approvePermission(req.request_id, 'admin', apiKey);
      setAuthorizingIds(prev => prev.filter(id => id !== reqId));
      setGrantedIds(prev => [...prev, reqId]);

      // Keep visible for 2 seconds to show success
      setTimeout(() => {
        setRequests(prev => prev.filter(r => r.request_id !== reqId));
        setGrantedIds(prev => prev.filter(id => id !== reqId));
      }, 2000);
    } catch (err) {
      setAuthorizingIds(prev => prev.filter(id => id !== reqId));
      setError(`CRITICAL: Authorization failed. ${err}`);
      console.error("Failed to grant permission:", err);
    }
  };

  const handleDeny = async (req: PermissionRequest) => {
    try {
      await api.denyPermission(req.request_id, 'admin', apiKey);
      setRequests(prev => prev.filter(r => r.request_id !== req.request_id));
      setError(null);
    } catch (err) {
      setError(`Failed to deny permission: ${err}`);
      console.error("Failed to deny permission:", err);
    }
  };

  if (requests.length === 0) return null;

  return (
    <div className="fixed bottom-24 right-8 z-[1000] flex flex-col gap-4 max-w-sm w-full animate-in slide-in-from-right-full duration-500">
      {error && (
        <div className="bg-red-500 text-white p-4 rounded-2xl shadow-lg flex items-center gap-3 animate-bounce">
          <AlertTriangle size={20} />
          <p className="text-xs font-bold uppercase tracking-tight">{error}</p>
          <button onClick={() => setError(null)} className="ml-auto"><X size={14} /></button>
        </div>
      )}

      {requests.map((req, idx) => {
        const reqId = req.request_id;
        const isAuthorizing = authorizingIds.includes(reqId);
        const isGranted = grantedIds.includes(reqId);

        return (
          <div
            key={req.request_id}
            className={`bg-white/90 backdrop-blur-2xl border rounded-[2rem] shadow-2xl overflow-hidden shadow-brand/20 flex flex-col transition-all duration-500 ${
              isGranted ? 'border-emerald-500 scale-95 opacity-50' : 'border-white'
            }`}
          >
            {/* Header */}
            <div className={`p-4 flex items-center justify-between text-white transition-colors duration-500 ${
              isGranted ? 'bg-emerald-500' : 'bg-brand'
            }`}>
              <div className="flex items-center gap-2">
                {isGranted ? <Check size={18} /> : <ShieldAlert size={18} />}
                <span className="text-[10px] font-black uppercase tracking-[0.2em]">
                  {isGranted ? 'Protocol Authorized' : 'Security Protocol'}
                </span>
              </div>
              {!isGranted && (
                <button onClick={() => handleDeny(req)} className="p-1 hover:bg-white/20 rounded-lg transition-colors">
                  <X size={16} />
                </button>
              )}
            </div>

            <div className="p-6">
              <div className="flex items-start gap-4 mb-4">
                <div className={`p-3 rounded-2xl transition-colors ${
                  isGranted ? 'bg-emerald-50 text-emerald-500' : 'bg-brand/10 text-brand'
                }`}>
                  {isGranted ? <Unlock size={20} /> : <Lock size={20} />}
                </div>
                <div>
                  <h4 className="text-sm font-black text-content-primary uppercase tracking-tight">
                    {isGranted ? 'Access Granted' : 'Access Request'}
                  </h4>
                  <p className="text-[10px] text-content-tertiary font-mono mt-0.5">{req.plugin_id}</p>
                </div>
              </div>

              <div className="bg-surface-base border border-edge-subtle rounded-xl p-3 mb-6">
                 <div className="flex items-center gap-2 mb-2">
                   <div className={`w-1.5 h-1.5 rounded-full ${isGranted ? 'bg-emerald-500' : 'bg-amber-500'}`} />
                   <span className="text-[9px] font-black text-content-secondary uppercase tracking-widest">Capability Status</span>
                 </div>
                 <p className="text-xs font-bold text-content-primary uppercase tracking-wide mb-2">{req.permission_type}</p>
                 {req.target_resource && (
                   <p className="text-[9px] text-content-tertiary font-mono mb-1">{req.target_resource}</p>
                 )}
                 <p className="text-[10px] text-content-secondary leading-relaxed italic">
                   {isGranted ? "Resource has been successfully injected into container." : `"${req.justification}"`}
                 </p>
              </div>

              {!isGranted && (
                <div className="flex gap-3">
                  <button 
                    disabled={isAuthorizing}
                    onClick={() => handleDeny(req)}
                    className="flex-1 py-2.5 rounded-xl border border-edge text-[10px] font-bold text-content-tertiary hover:text-content-secondary hover:bg-surface-base transition-all uppercase tracking-widest disabled:opacity-30"
                  >
                    Deny
                  </button>
                  <button 
                    disabled={isAuthorizing}
                    onClick={() => handleGrant(req)}
                    className="flex-1 py-2.5 rounded-xl bg-brand text-white text-[10px] font-bold shadow-lg shadow-brand/30 hover:scale-[1.02] active:scale-95 transition-all uppercase tracking-widest flex items-center justify-center gap-2 disabled:opacity-50"
                  >
                    {isAuthorizing ? (
                      <div className="w-3 h-3 border-2 border-white/20 border-t-white rounded-full animate-spin" />
                    ) : <Unlock size={14} />}
                    {isAuthorizing ? 'Authorizing...' : 'Authorize'}
                  </button>
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
