import { useState, useCallback } from 'react';

const SESSION_KEY = 'exiv-api-key';
const LOCAL_KEY   = 'exiv-api-key-local';

function readStored(): string {
  try {
    return localStorage.getItem(LOCAL_KEY) || sessionStorage.getItem(SESSION_KEY) || '';
  } catch {
    return '';
  }
}

function isLocalPersisted(): boolean {
  try { return !!localStorage.getItem(LOCAL_KEY); } catch { return false; }
}

export interface ApiKeyHookValue {
  apiKey: string;
  isPersisted: boolean;
  setApiKey: (key: string) => void;
  setPersist: (persist: boolean) => void;
  forgetApiKey: () => void;
}

export function useApiKeyProvider(): ApiKeyHookValue {
  const [apiKey, setApiKeyState] = useState<string>(readStored);
  const [isPersisted, setIsPersistedState] = useState<boolean>(isLocalPersisted);

  const setApiKey = useCallback((key: string) => {
    setApiKeyState(key);
    try {
      if (isPersisted) {
        localStorage.setItem(LOCAL_KEY, key);
      } else {
        sessionStorage.setItem(SESSION_KEY, key);
        localStorage.removeItem(LOCAL_KEY);
      }
    } catch { /* storage unavailable */ }
  }, [isPersisted]);

  const setPersist = useCallback((persist: boolean) => {
    setIsPersistedState(persist);
    try {
      if (persist && apiKey) {
        localStorage.setItem(LOCAL_KEY, apiKey);
      } else {
        localStorage.removeItem(LOCAL_KEY);
      }
    } catch { /* storage unavailable */ }
  }, [apiKey]);

  const forgetApiKey = useCallback(() => {
    setApiKeyState('');
    setIsPersistedState(false);
    try {
      localStorage.removeItem(LOCAL_KEY);
      sessionStorage.removeItem(SESSION_KEY);
    } catch { /* storage unavailable */ }
  }, []);

  return { apiKey, isPersisted, setApiKey, setPersist, forgetApiKey };
}
