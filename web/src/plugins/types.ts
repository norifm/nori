// A plugin as the page holds it. `WebPlugin` is the core's `read_web_plugin`.

export type RowKind = "Switch" | "Choice" | "Level" | "Secret" | "Text" | "Action" | "Status" | "Section" | "Note" | "Link";

export type Row = {
  id: string;
  kind: RowKind;
  numeric: boolean;
  label: string;
  options: string[];
  min: number | null;
  max: number | null;
  step: number | null;
  default: number | null;
  hint: string | null;
  unit: string | null;
  decimals: number;
  url: string | null;
};

export type WebPlugin = {
  id: string;
  version: string;
  name: string;
  about: string | null;
  points: string[];
  stage: boolean;
  net: string[];
  rows: Row[];
  blocks: string[];
};

/** A value a row holds: a number for a numeric row, else text. */
export type Value = number | string;

/** What the installed list keeps of one plugin; its code is kept apart (`db.ts`). */
export type Installed = { plugin: WebPlugin; on: boolean; values: Record<string, Value>; icon?: string };

/** The transpiled component: its glue and its core modules by name. */
export type Code = { js: string; cores: Record<string, Uint8Array> };

/** A row's value as the plugin API's `value`. */
export function witValue(row: Row, v: Value) {
  if (row.kind === "Switch") return { tag: "switch", val: v !== 0 };
  if (row.kind === "Choice") return { tag: "choice", val: Number(v) };
  if (row.kind === "Level") return { tag: "level", val: Number(v) };
  return { tag: "text", val: String(v) };
}

/** Its value, or its default. */
export const valueOf = (row: Row, values: Record<string, Value>): Value | undefined => values[row.id] ?? row.default ?? undefined;

/** Its numeric rows' values, in order, for a sound stage. */
export const numbers = (i: Installed) => i.plugin.rows.filter((r) => r.numeric).map((r) => Number(valueOf(r, i.values) ?? 0));
