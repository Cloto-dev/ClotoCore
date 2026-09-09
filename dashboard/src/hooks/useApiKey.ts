import { useCallback } from 'react';
import { API_KEY_STORAGE_KEY, endBrowserSession } from '../services/session';
import { useSessionStorage } from './useStorage';

export interface ApiKeyHookValue {
  apiKey: string;
  setApiKey: (key: string) => void;
  forgetApiKey: () => void;
}

export function useApiKeyProvider(): ApiKeyHookValue {
  const [apiKey, setApiKey, forgetStoredKey] = useSessionStorage(API_KEY_STORAGE_KEY, '');

  // Ending the session is part of forgetting the key, not a separate button a
  // call site could omit: a browser that has dropped the key while still
  // holding a live admin cookie has not signed out, it has only lost the
  // ability to say so.
  const forgetApiKey = useCallback(() => {
    forgetStoredKey();
    void endBrowserSession();
  }, [forgetStoredKey]);

  return { apiKey, setApiKey, forgetApiKey };
}
