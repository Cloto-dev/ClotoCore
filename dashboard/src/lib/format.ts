/**
 * Human-readable byte size. Shared by the health section (database size) and
 * the Danger Zone (purge plan sizes) so the two cannot disagree about what
 * "1.5 MB" means.
 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Strip the namespace prefix (e.g. "mind.", "memory.") from a server ID for display. */
export function displayServerId(id: string): string {
  const dot = id.indexOf('.');
  return dot >= 0 ? id.slice(dot + 1) : id;
}
