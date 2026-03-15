import { useCallback, useState } from 'react';
import { extractError } from '../lib/errors';

/**
 * Generic async action wrapper with loading and error state.
 *
 * Usage:
 *   const action = useAsyncAction('Failed to save');
 *   const handleSave = () => action.run(async () => { await api.save(...); });
 *   // action.isLoading, action.error available for UI
 */
export function useAsyncAction(errorFallback = 'Operation failed') {
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(
    async <T>(fn: () => Promise<T>): Promise<T | undefined> => {
      setIsLoading(true);
      setError(null);
      try {
        const result = await fn();
        return result;
      } catch (err) {
        setError(extractError(err, errorFallback));
        return undefined;
      } finally {
        setIsLoading(false);
      }
    },
    [errorFallback],
  );

  const clearError = useCallback(() => setError(null), []);

  return { run, isLoading, error, clearError };
}
