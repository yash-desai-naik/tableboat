/** Cleared when the connection changes; SqlEditor registers its cache here. */
export const schemaCacheClearers = new Set<() => void>();

export function clearSchemaCache() {
  for (const clear of schemaCacheClearers) clear();
}
