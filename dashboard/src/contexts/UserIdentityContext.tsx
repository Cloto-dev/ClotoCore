import React, { createContext, useContext } from 'react';
import { useUserIdentityProvider, UserIdentityHookValue } from '../hooks/useUserIdentity';

const UserIdentityContext = createContext<UserIdentityHookValue | null>(null);

export function UserIdentityProvider({ children }: { children: React.ReactNode }) {
  const value = useUserIdentityProvider();
  return <UserIdentityContext.Provider value={value}>{children}</UserIdentityContext.Provider>;
}

export function useUserIdentity(): UserIdentityHookValue {
  const ctx = useContext(UserIdentityContext);
  if (!ctx) throw new Error('useUserIdentity must be used within UserIdentityProvider');
  return ctx;
}
