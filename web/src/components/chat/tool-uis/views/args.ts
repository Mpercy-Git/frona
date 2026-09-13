/**
 * Read a "one or many" tool argument — the mirror of `str_list_arg` in
 * crates/frona-server/src/tool/mod.rs.
 *
 * The batchable tools (`read`, `memory_search`, `memory_remember`, `memory_cite`,
 * `store_user_memory`, `store_agent_memory`) each accept a singular string or a plural
 * array, so a row that only looked at the singular key renders blank the moment the
 * agent batches. Keep the two in lockstep: blanks dropped, duplicates collapsed, order
 * preserved.
 */
export function strListArg(
  args: unknown,
  singular: string,
  plural: string,
): string[] {
  const a = (args && typeof args === "object" ? args : {}) as Record<string, unknown>;
  const out: string[] = [];
  const push = (value: unknown) => {
    if (typeof value !== "string") return;
    const trimmed = value.trim();
    if (trimmed.length > 0 && !out.includes(trimmed)) out.push(trimmed);
  };
  for (const key of [singular, plural]) {
    const value = a[key];
    if (Array.isArray(value)) value.forEach(push);
    else push(value);
  }
  return out;
}

/** `a.md` for one, `a.md +2 more` for a batch — a subtitle stays one line. */
export function summariseList(items: string[]): string | null {
  if (items.length === 0) return null;
  if (items.length === 1) return items[0];
  return `${items[0]} +${items.length - 1} more`;
}
