import React, { createContext, useContext } from 'react';
import { useApiKeyProvider, ApiKeyHookValue } from '../hooks/useApiKey';

const ApiKeyContext = createContext<ApiKeyHookValue | null>(null);

export function ApiKeyProvider({ children }: { children: React.ReactNode }) {
  const value = useApiKeyProvider();
  return <ApiKeyContext.Provider value={value}>{children}</ApiKeyContext.Provider>;
}

export function useApiKey(): ApiKeyHookValue {
  const ctx = useContext(ApiKeyContext);
  if (!ctx) throw new Error('useApiKey must be used within ApiKeyProvider');
  return ctx;
}
