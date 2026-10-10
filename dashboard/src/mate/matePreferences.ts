import { useCallback } from 'react';
import { useLocalStorage } from '../hooks/useStorage';

export type ChatSize = 'side' | 'expanded';
export type GlassProfile = 'soft' | 'crisp' | 'still';

export const CHAT_SIZES: readonly ChatSize[] = ['side', 'expanded'];
export const GLASS_PROFILES: readonly GlassProfile[] = ['soft', 'crisp', 'still'];

// How this screen looks on this device, not who the partner is: the same partner can be open on
// two devices at once, so these stay in the browser beside the theme instead of in agent metadata.
export const CHAT_SIZE_KEY = 'cloto-mate-chat-size';
export const GLASS_KEY = 'cloto-mate-glass';

/** A stored value the screen no longer offers (or a hand-edited one) falls back to the default. */
function allowed<T extends string>(value: string, choices: readonly T[], fallback: T): T {
  return (choices as readonly string[]).includes(value) ? (value as T) : fallback;
}

export function useMatePreferences() {
  const [storedSize, storeSize] = useLocalStorage(CHAT_SIZE_KEY, 'side');
  const [storedGlass, storeGlass] = useLocalStorage(GLASS_KEY, 'soft');
  const setChatSize = useCallback((size: ChatSize) => storeSize(size), [storeSize]);
  const setGlass = useCallback((glass: GlassProfile) => storeGlass(glass), [storeGlass]);
  return {
    chatSize: allowed(storedSize, CHAT_SIZES, 'side'),
    setChatSize,
    glass: allowed(storedGlass, GLASS_PROFILES, 'soft'),
    setGlass,
  };
}
