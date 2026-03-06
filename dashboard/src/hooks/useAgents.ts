import { useCallback } from 'react';
import { api } from '../services/api';
import { useRemoteData } from './useRemoteData';
import { useApiKey } from '../contexts/ApiKeyContext';

export function useAgents() {
  const { apiKey } = useApiKey();
  const fetcher = useCallback(() => api.getAgents(apiKey), [apiKey]);
  const { data: agents, ...rest } = useRemoteData(
    fetcher,
    { key: 'agents', errorMessage: 'Failed to fetch agents' },
  );
  return { agents, ...rest };
}
