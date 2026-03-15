import { useApi } from './useApi';
import { useRemoteData } from './useRemoteData';

export function useMcpServers() {
  const api = useApi();
  const { data: servers, ...rest } = useRemoteData(() => api.listMcpServers().then((d) => d.servers), {
    key: `mcp-servers:${api.apiKey}`,
    errorMessage: 'Failed to connect',
    minRefetchMs: 400,
  });
  return { servers, ...rest };
}
