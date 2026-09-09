import { useApi } from './useApi';
import { useRemoteData } from './useRemoteData';

/** UI modules the kernel found under `<data_dir>/modules/`.
 *
 * The list includes directories the kernel rejected (they carry `error` and no
 * `name`), so callers that render a menu have to filter; callers that render a
 * diagnostic should not. */
export function useModules() {
  const api = useApi();
  const { data: modules, ...rest } = useRemoteData(() => api.listModules(), {
    key: `modules:${api.apiKey}`,
    errorMessage: 'Failed to list modules',
    minRefetchMs: 400,
  });
  return { modules, ...rest };
}
