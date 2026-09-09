import type React from 'react';
import { createContext, useContext, useEffect } from 'react';
import { type ApiKeyHookValue, useApiKeyProvider } from '../hooks/useApiKey';
import { getAutoApiKey } from '../lib/tauri';
import { startBrowserSession } from '../services/session';

const ApiKeyContext = createContext<ApiKeyHookValue | null>(null);

export function ApiKeyProvider({ children }: { children: React.ReactNode }) {
  const value = useApiKeyProvider();

  // In Tauri mode, auto-fetch the generated API key on mount
  useEffect(() => {
    if (value.apiKey) return; // already has a key in sessionStorage
    getAutoApiKey().then((key) => {
      if (key) value.setApiKey(key);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value.apiKey, value.setApiKey]); // run once on mount

  // Whenever the key changes — the user typed one, the wizard finished, it was
  // rotated — the browser needs a session standing on the new one. `main.tsx`
  // covers the already-stored case before anything mounts; this covers the rest.
  // Idempotent per key, so a re-render does not re-mint.
  useEffect(() => {
    if (value.apiKey) startBrowserSession(value.apiKey);
  }, [value.apiKey]);

  return <ApiKeyContext.Provider value={value}>{children}</ApiKeyContext.Provider>;
}

export function useApiKey(): ApiKeyHookValue {
  const ctx = useContext(ApiKeyContext);
  if (!ctx) throw new Error('useApiKey must be used within ApiKeyProvider');
  return ctx;
}
