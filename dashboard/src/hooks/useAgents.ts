import { useCallback } from 'react';
import { useRemoteData } from './useRemoteData';
import { useApi } from './useApi';

export function useAgents() {
  const api = useApi();
  const fetcher = useCallback(() => api.getAgents(), [api]);
  const { data: agents, ...rest } = useRemoteData(
    fetcher,
    { key: `agents:${api.apiKey}`, errorMessage: 'Failed to fetch agents' },
  );
  return { agents, ...rest };
}
