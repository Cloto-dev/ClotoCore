/** Extract a human-readable message from an unknown error value. */
export function extractError(err: unknown, fallback: string): string {
  return err instanceof Error ? err.message : fallback;
}
