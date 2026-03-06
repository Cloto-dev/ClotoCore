import { useSessionStorage } from './useStorage';

export interface ApiKeyHookValue {
  apiKey: string;
  setApiKey: (key: string) => void;
  forgetApiKey: () => void;
}

export function useApiKeyProvider(): ApiKeyHookValue {
  const [apiKey, setApiKey, forgetApiKey] = useSessionStorage('cloto-api-key', '');
  return { apiKey, setApiKey, forgetApiKey };
}
