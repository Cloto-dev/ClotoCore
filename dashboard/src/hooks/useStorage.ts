import { useState, useCallback } from 'react';

function createStorageHook(getStorage: () => Storage) {
  return function useStorage(key: string, fallback: string): [string, (v: string) => void, () => void] {
    const [value, setInner] = useState<string>(() => {
      try { return getStorage().getItem(key) ?? fallback; } catch { return fallback; }
    });

    const set = useCallback((v: string) => {
      setInner(v);
      try { getStorage().setItem(key, v); } catch {}
    }, [key]);

    const remove = useCallback(() => {
      setInner(fallback);
      try { getStorage().removeItem(key); } catch {}
    }, [key, fallback]);

    return [value, set, remove];
  };
}

/** Read/write a string value in localStorage with error handling. */
export const useLocalStorage = createStorageHook(() => localStorage);

/** Read/write a string value in sessionStorage with error handling. */
export const useSessionStorage = createStorageHook(() => sessionStorage);
