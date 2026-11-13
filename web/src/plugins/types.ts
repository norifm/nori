// A plugin as the page holds it. `WebPlugin` is the core's `read_web_plugin`.

export type RowKind = "Switch" | "Choice" | "Level" | "Secret" | "Text" | "Action" | "Status" | "Section" | "Note" | "Link" | "Flags" | "Page" | "Segments" | "Meter";

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
  /** The id of the page row it sits on; null for the plugin's own page. */
  page: string | null;
};

export type WebPlugin = {
  id: string;
  version: string;
  name: string;
  about: string | null;
  points: string[];
  stage: boolean;
  net: string[];
  overrides: string[];
  /** Absent from a plugin kept before they were read. */
  permissions?: string[];
  rows: Row[];
  blocks: string[];
};

/** A package read, not yet installed; `key`, its author's as its repo lists it (none for a file). */
export type Read = { plugin: WebPlugin; wasm: Uint8Array; icon?: string; readme?: string; key?: string };

export type Fit = { state: "Supported" | "Unknown" } | { state: "Blocked"; blocks: string[] };
/** `key` and `signature`: its author's ed25519 key and signature, both or neither. */
export type Version = { version: string; released: string | null; url: string; sha256: string; key: string | null; signature: string | null };

/** Why a package doesn't install: it isn't the one listed, its signature doesn't verify, or its plugin
 * pinned another author's key (`package::checked`, `package::admits`). */
export type Refusal = "NotListed" | "Signature" | "OtherKey";

/** A package that doesn't install, and why. */
export class Refused extends Error {
  constructor(readonly why: Refusal) {
    super(`refused: ${why}`);
  }
}
/** What the repo lists of a plugin (`plugin_offers`): `waiting`, an update for one not kept at its version. */
export type Offer = {
  id: string;
  name: string;
  about: string | null;
  icon: string | null;
  readme: string | null;
  source: string | null;
  versions: Version[];
  state: "New" | "Update" | "Installed";
  waiting: boolean;
  fit: Fit;
};

/** A block of a README, `plugin_readme`'s. */
export type Readme = { kind: "Heading"; level: number; text: string } | { kind: "Text" | "Item" | "Code"; text: string };

/** A value a row holds: a number for a numeric row, else text. */
export type Value = number | string;

/** What the installed list keeps of one plugin; its code is kept apart (`db.ts`). */
export type Installed = {
  plugin: WebPlugin;
  on: boolean;
  values: Record<string, Value>;
  icon?: string;
  readme?: string;
  /** Kept at its version: no update is offered. */
  held?: boolean;
  fromFile?: boolean;
  /** The key its first signed version pinned: every version after must be signed by it. */
  key?: string;
  /** Its place in the order plugins apply in. */
  at?: number;
};

/** The transpiled component: its glue and its core modules by name. */
export type Code = { js: string; cores: Record<string, Uint8Array> };

/** A row's value as the plugin API's `value`. */
export function witValue(row: Row, v: Value) {
  if (row.kind === "Switch") return { tag: "switch", val: v !== 0 };
  if (row.kind === "Choice" || row.kind === "Segments") return { tag: "choice", val: Number(v) };
  if (row.kind === "Level") return { tag: "level", val: Number(v) };
  return { tag: "text", val: String(v) };
}

/** Its value, or its default. */
export const valueOf = (row: Row, values: Record<string, Value>): Value | undefined => values[row.id] ?? row.default ?? undefined;

/** Its numeric rows' values, in order, for a sound stage. */
export const numbers = (i: Installed) => i.plugin.rows.filter((r) => r.numeric).map((r) => Number(valueOf(r, i.values) ?? 0));
