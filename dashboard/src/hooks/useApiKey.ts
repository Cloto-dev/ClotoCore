import { useState, useCallback } from 'react';

const SESSION_KEY = 'cloto-api-key';

function readStored(): string {
  try {
    return sessionStorage.getItem(SESSION_KEY) || '';
  } catch {
    return '';
  }
}

export interface ApiKeyHookValue {
  apiKey: string;
  setApiKey: (key: string) => void;
  forgetApiKey: () => void;
}

export function useApiKeyProvider(): ApiKeyHookValue {
  const [apiKey, setApiKeyState] = useState<string>(readStored);

  const setApiKey = useCallback((key: string) => {
    setApiKeyState(key);
    try {
      sessionStorage.setItem(SESSION_KEY, key);
    } catch { /* storage unavailable */ }
  }, []);

  const forgetApiKey = useCallback(() => {
    setApiKeyState('');
    try {
      sessionStorage.removeItem(SESSION_KEY);
    } catch { /* storage unavailable */ }
  }, []);

  return { apiKey, setApiKey, forgetApiKey };
}
