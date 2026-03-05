import { useState, useCallback } from 'react';

const STORAGE_KEY_ID = 'cloto-user-id';
const STORAGE_KEY_NAME = 'cloto-user-name';
const DEFAULT_ID = 'default';
const DEFAULT_NAME = 'User';

export interface UserIdentity {
  id: string;
  name: string;
}

export interface UserIdentityHookValue {
  identity: UserIdentity;
  setIdentity: (id: string, name: string) => void;
}

export function useUserIdentityProvider(): UserIdentityHookValue {
  const [identity, setIdentityState] = useState<UserIdentity>(() => ({
    id: localStorage.getItem(STORAGE_KEY_ID) || DEFAULT_ID,
    name: localStorage.getItem(STORAGE_KEY_NAME) || DEFAULT_NAME,
  }));

  const setIdentity = useCallback((id: string, name: string) => {
    const safeId = id.trim() || DEFAULT_ID;
    const safeName = name.trim() || DEFAULT_NAME;
    try {
      localStorage.setItem(STORAGE_KEY_ID, safeId);
      localStorage.setItem(STORAGE_KEY_NAME, safeName);
    } catch { /* storage unavailable */ }
    setIdentityState({ id: safeId, name: safeName });
  }, []);

  return { identity, setIdentity };
}
