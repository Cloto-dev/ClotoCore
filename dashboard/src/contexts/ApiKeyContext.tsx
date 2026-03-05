import React, { createContext, useContext, useEffect } from 'react';
import { useApiKeyProvider, ApiKeyHookValue } from '../hooks/useApiKey';
import { getAutoApiKey } from '../lib/tauri';

const ApiKeyContext = createContext<ApiKeyHookValue | null>(null);

export function ApiKeyProvider({ children }: { children: React.ReactNode }) {
  const value = useApiKeyProvider();

  // In Tauri mode, auto-fetch the generated API key on mount
  useEffect(() => {
    if (value.apiKey) return; // already has a key in sessionStorage
    getAutoApiKey().then(key => {
      if (key) value.setApiKey(key);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // run once on mount

  return <ApiKeyContext.Provider value={value}>{children}</ApiKeyContext.Provider>;
}

export function useApiKey(): ApiKeyHookValue {
  const ctx = useContext(ApiKeyContext);
  if (!ctx) throw new Error('useApiKey must be used within ApiKeyProvider');
  return ctx;
}
