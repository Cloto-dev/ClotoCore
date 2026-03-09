/** Strip the namespace prefix (e.g. "mind.", "memory.") from a server ID for display. */
export function displayServerId(id: string): string {
  const dot = id.indexOf('.');
  return dot >= 0 ? id.slice(dot + 1) : id;
}
