import React, { useState, useRef, useEffect } from 'react';
import { Lock, Key, X, Eye, EyeOff, AlertTriangle, CheckCircle2, Loader } from 'lucide-react';
import { useApiKey } from '../contexts/ApiKeyContext';
import { api } from '../services/api';

export function ApiKeyGate() {
  const { apiKey, isPersisted, setApiKey, setPersist, forgetApiKey } = useApiKey();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState('');
  const [show, setShow] = useState(false);
  const [validating, setValidating] = useState(false);
  const [validationError, setValidationError] = useState('');
  const [confirmInvalidate, setConfirmInvalidate] = useState(false);
  const [invalidating, setInvalidating] = useState(false);
  const [invalidateError, setInvalidateError] = useState('');
  const panelRef = useRef<HTMLDivElement>(null);

  const isSet = !!apiKey;

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        setOpen(false);
        setConfirmInvalidate(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const handleOpen = () => {
    setDraft(apiKey);
    setShow(false);
    setConfirmInvalidate(false);
    setInvalidateError('');
    setValidationError('');
    setOpen(true);
  };

  const handleSave = async () => {
    const key = draft.trim();
    if (!key) return;
    setValidating(true);
    setValidationError('');
    try {
      // Validate by calling an admin endpoint with this key
      await api.applyPluginSettings([], key);
      setApiKey(key);
      setOpen(false);
    } catch {
      setValidationError('キーがサーバーと一致しません。EXIV_API_KEY の値を確認してください。');
    } finally {
      setValidating(false);
    }
  };

  const handleForget = () => {
    forgetApiKey();
    setDraft('');
    setOpen(false);
  };

  const handleInvalidate = async () => {
    if (!apiKey) return;
    setInvalidating(true);
    setInvalidateError('');
    try {
      await api.invalidateApiKey(apiKey);
      forgetApiKey();
      setOpen(false);
      setConfirmInvalidate(false);
    } catch (e: any) {
      setInvalidateError(e?.message || 'Failed to invalidate key');
    } finally {
      setInvalidating(false);
    }
  };

  return (
    <div className="relative" ref={panelRef}>
      {/* Trigger button */}
      <button
        onClick={handleOpen}
        className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-mono font-bold transition-all ${
          isSet
            ? 'bg-green-500/10 text-green-400 border border-green-500/20 hover:bg-green-500/20'
            : 'bg-amber-500/10 text-amber-400 border border-amber-500/20 hover:bg-amber-500/20'
        }`}
        title={isSet ? 'API Key is set' : 'Set API Key'}
      >
        {isSet ? <Key size={11} /> : <Lock size={11} />}
        {isSet ? '●●●●●●●●' : 'API Key'}
        {isPersisted && <span className="text-[9px] opacity-60">SAVED</span>}
      </button>

      {/* Dropdown panel */}
      {open && (
        <div className="absolute right-0 top-full mt-2 w-72 bg-surface-primary border border-edge rounded-2xl shadow-2xl z-50 p-4 space-y-3 animate-in zoom-in-95 duration-150">
          <div className="flex items-center justify-between">
            <span className="text-xs font-bold text-content-primary flex items-center gap-1.5">
              <Key size={12} className="text-brand" /> Admin API Key
            </span>
            <button onClick={() => setOpen(false)} className="text-content-muted hover:text-content-primary">
              <X size={14} />
            </button>
          </div>

          {/* Key input */}
          <div className="relative">
            <Lock size={11} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-content-muted" />
            <input
              type={show ? 'text' : 'password'}
              value={draft}
              onChange={e => setDraft(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && handleSave()}
              placeholder="Enter API key..."
              autoFocus
              className="w-full pl-7 pr-8 py-2 rounded-lg border border-edge text-xs font-mono text-content-primary bg-surface-base placeholder:text-content-muted focus:outline-none focus:border-brand"
            />
            <button
              onClick={() => setShow(s => !s)}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-content-muted hover:text-content-primary"
            >
              {show ? <EyeOff size={12} /> : <Eye size={12} />}
            </button>
          </div>

          {/* Persist toggle */}
          <label className="flex items-center gap-2 cursor-pointer select-none">
            <div
              onClick={() => setPersist(!isPersisted)}
              className={`w-8 h-4 rounded-full transition-colors ${isPersisted ? 'bg-brand' : 'bg-surface-secondary'} relative`}
            >
              <div className={`absolute top-0.5 w-3 h-3 rounded-full bg-white shadow transition-transform ${isPersisted ? 'translate-x-4' : 'translate-x-0.5'}`} />
            </div>
            <span className="text-[10px] text-content-secondary">この端末で記憶する (localStorage)</span>
          </label>

          {/* Validation error */}
          {validationError && (
            <div className="flex items-start gap-1.5 p-2 bg-red-500/10 rounded-lg border border-red-500/20">
              <AlertTriangle size={11} className="text-red-400 flex-shrink-0 mt-0.5" />
              <span className="text-[10px] text-red-400">{validationError}</span>
            </div>
          )}

          {/* Actions */}
          <div className="flex gap-2">
            <button
              onClick={handleSave}
              disabled={!draft.trim() || validating}
              className="flex-1 py-1.5 rounded-lg bg-brand text-white text-xs font-bold disabled:opacity-40 hover:bg-brand/90 transition-colors flex items-center justify-center gap-1"
            >
              {validating ? <Loader size={11} className="animate-spin" /> : <CheckCircle2 size={11} />}
              {validating ? '確認中...' : '確認して保存'}
            </button>
            {isSet && (
              <button
                onClick={handleForget}
                className="px-3 py-1.5 rounded-lg border border-edge text-xs text-content-secondary hover:text-red-400 hover:border-red-400/40 transition-colors"
                title="セッションからキーを削除"
              >
                削除
              </button>
            )}
          </div>

          {/* Invalidate section */}
          {isSet && !confirmInvalidate && (
            <button
              onClick={() => setConfirmInvalidate(true)}
              className="w-full py-1.5 rounded-lg border border-red-500/20 text-red-400 text-[10px] font-bold hover:bg-red-500/10 transition-colors"
            >
              このキーをシステム全体で無効化...
            </button>
          )}

          {isSet && confirmInvalidate && (
            <div className="p-3 bg-red-500/10 rounded-lg border border-red-500/20 space-y-2">
              <div className="flex items-start gap-2 text-red-400 text-[10px]">
                <AlertTriangle size={12} className="flex-shrink-0 mt-0.5" />
                <span>このキーはExivシステム全体で永久に使用不可になります。<br />新しいキーで再起動するまで管理操作ができなくなります。</span>
              </div>
              {invalidateError && <p className="text-[10px] text-red-400">{invalidateError}</p>}
              <div className="flex gap-2">
                <button
                  onClick={() => setConfirmInvalidate(false)}
                  className="flex-1 py-1 rounded-lg border border-edge text-[10px] text-content-secondary hover:bg-surface-secondary transition-colors"
                >
                  キャンセル
                </button>
                <button
                  onClick={handleInvalidate}
                  disabled={invalidating}
                  className="flex-1 py-1 rounded-lg bg-red-600 text-white text-[10px] font-bold hover:bg-red-700 disabled:opacity-50 transition-colors"
                >
                  {invalidating ? '処理中...' : '無効化する'}
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
