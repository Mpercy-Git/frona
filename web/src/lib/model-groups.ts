/**
 * Prettify a model-group key (a snake_case "Group ID") for display:
 * `"my_group"` → `"My Group"`, `"primary"` → `"Primary"`.
 */
export function formatGroupName(name: string): string {
  return name
    .split("_")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}
