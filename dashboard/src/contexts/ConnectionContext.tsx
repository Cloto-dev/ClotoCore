import React, { createContext, useContext } from 'react';
import { useConnectionStatusProvider, ConnectionStatus } from '../hooks/useConnectionStatus';

const ConnectionContext = createContext<ConnectionStatus | null>(null);

export function ConnectionProvider({ children }: { children: React.ReactNode }) {
  const value = useConnectionStatusProvider();
  return <ConnectionContext.Provider value={value}>{children}</ConnectionContext.Provider>;
}

export function useConnection(): ConnectionStatus {
  const ctx = useContext(ConnectionContext);
  if (!ctx) throw new Error('useConnection must be used within ConnectionProvider');
  return ctx;
}
