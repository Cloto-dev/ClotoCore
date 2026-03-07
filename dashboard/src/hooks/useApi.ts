import { useMemo } from 'react';
import { useApiKey } from '../contexts/ApiKeyContext';
import { createAuthenticatedApi, type AuthenticatedApi } from '../services/api';

/** API methods with pre-bound apiKey. Also exposes raw `apiKey` for SSE/custom hooks. */
export function useApi(): AuthenticatedApi & { apiKey: string } {
  const { apiKey } = useApiKey();
  const key = apiKey || '';
  return useMemo(() => ({ ...createAuthenticatedApi(key), apiKey: key }), [key]);
}
