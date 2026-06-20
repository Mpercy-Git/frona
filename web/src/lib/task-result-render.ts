// Mirror of Rust `chat::channel::render::render_result_markdown`
// (crates/frona-server/src/chat/channel/render.rs). Keep in lockstep.

import type { MessageResponse, MessageEvent } from "./types";

type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

export function renderMessageBody(msg: MessageResponse): string {
  const event: MessageEvent | undefined = msg.event;
  if (event?.type !== "TaskCompletion") {
    return msg.content || "";
  }
  const schema = event.data.schema;
  if (!schema) {
    return msg.content || "";
  }
  let parsed: JsonValue;
  try {
    parsed = JSON.parse(msg.content || "");
  } catch {
    return msg.content || "";
  }
  // No fallback to msg.content on null — that's the signal to suppress the
  // bubble for complex schemas without a summary field.
  return renderResultMarkdown(schema as JsonValue, parsed) ?? "";
}

export function renderResultMarkdown(
  schema: JsonValue,
  value: JsonValue,
): string | null {
  if (value === null) return null;
  if (Array.isArray(value)) {
    if (value.length === 0) return null;
    return value.map((v) => `- ${renderValueMd(v)}`).join("\n");
  }
  if (typeof value === "object") {
    const obj = value as { [key: string]: JsonValue };
    const keys = Object.keys(obj);
    if (keys.length === 0) return null;
    if (isComplexObject(obj)) {
      return renderComplexObject(schema, obj);
    }
    const props = findObjectProperties(schema);
    const lines: Array<[string, string]> = [];
    if (props) {
      for (const [key, propSchema] of Object.entries(props)) {
        const v = obj[key];
        if (v !== undefined && v !== null) {
          const label = readDescription(propSchema) ?? key;
          lines.push([label, renderValueMd(v)]);
        }
      }
    } else {
      for (const key of keys) {
        const v = obj[key];
        if (v !== null) {
          lines.push([key, renderValueMd(v)]);
        }
      }
    }
    if (lines.length === 0) return null;
    if (lines.length === 1) return lines[0][1];
    return lines.map(([label, val]) => `**${label}**: ${val}`).join("\n");
  }
  return renderValueMd(value);
}

// Top-level object is "complex" when any of its values is itself an object
// or contains objects. Complex schemas must include a top-level `summary`
// string property; the renderer surfaces only that field.
const COMPLEX_RENDER_KEY = "summary";

function isComplexObject(obj: { [key: string]: JsonValue }): boolean {
  return Object.values(obj).some(valueIsNonScalar);
}

function valueIsNonScalar(v: JsonValue): boolean {
  if (v === null) return false;
  if (typeof v === "object" && !Array.isArray(v)) return true;
  if (Array.isArray(v)) return v.some(valueIsNonScalar);
  return false;
}

function renderComplexObject(
  _schema: JsonValue,
  obj: { [key: string]: JsonValue },
): string | null {
  const v = obj[COMPLEX_RENDER_KEY];
  return typeof v === "string" && v.length > 0 ? v : null;
}

function findObjectProperties(
  schema: JsonValue,
): { [key: string]: JsonValue } | null {
  if (!isObject(schema)) return null;
  const props = (schema as { properties?: JsonValue }).properties;
  if (isObject(props)) return props as { [key: string]: JsonValue };
  const oneOf = (schema as { oneOf?: JsonValue }).oneOf;
  const anyOf = (schema as { anyOf?: JsonValue }).anyOf;
  const branches = Array.isArray(oneOf) ? oneOf : Array.isArray(anyOf) ? anyOf : null;
  if (branches) {
    for (const branch of branches) {
      if (isObject(branch)) {
        const p = (branch as { properties?: JsonValue }).properties;
        if (isObject(p)) return p as { [key: string]: JsonValue };
      }
    }
  }
  return null;
}

function readDescription(schema: JsonValue): string | null {
  if (!isObject(schema)) return null;
  const d = (schema as { description?: unknown }).description;
  return typeof d === "string" ? d : null;
}

function renderValueMd(v: JsonValue): string {
  if (v === null) return "";
  if (typeof v === "string") return v;
  if (typeof v === "number") return String(v);
  if (typeof v === "boolean") return String(v);
  if (Array.isArray(v)) return v.map(renderValueMd).join(", ");
  return "```json\n" + JSON.stringify(v, null, 2) + "\n```";
}

function isObject(v: unknown): v is { [key: string]: JsonValue } {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}
