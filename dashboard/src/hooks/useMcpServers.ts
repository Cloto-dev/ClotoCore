import { useRemoteData } from './useRemoteData';
import { useApi } from './useApi';

export function useMcpServers() {
  const api = useApi();
  const { data: servers, ...rest } = useRemoteData(
    () => api.listMcpServers().then(d => d.servers),
    { key: `mcp-servers:${api.apiKey}`, errorMessage: 'Failed to connect', minRefetchMs: 400 },
  );
  return { servers, ...rest };
}
