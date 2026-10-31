// One plugin on its own thread. A plugin's host calls block, so its HTTP is synchronous XHR, which only a
// worker may do. It says what changed to the page, which keeps it (`host.svelte.ts`).

import type { Code, Row, Value } from "./types";
import { valueOf, witValue } from "./types";
import { wasi } from "./wasi";

/** The first message: the plugin to run. `proxy`: the address that carries its requests, if the page's
 * server has one. */
export type Start = { code: Code; net: string[]; proxy: string | null; rows: Row[]; values: Record<string, Value>; data: Map<string, Uint8Array> };
/** A call on one of its exports: `at` the interface, `call` the function. */
export type Call = { n: number; at: "lifecycle" | "hooks" | "slots"; call: string; args: unknown[] };

type Header = { name: string; value: string };
type HttpRequest = { method: string; url: string; headers: Header[]; body?: Uint8Array; timeoutMs?: number };
type HttpResponse = { status: number; headers: Header[]; body: Uint8Array };
type Answer = { ok: HttpResponse } | { failed: { kind: string; detail: string } };

const scope = self as unknown as { postMessage(m: unknown): void; onmessage: ((e: MessageEvent) => void) | null };
const decline = (kind: string, detail: string) => ({ payload: { kind, detail } });

/** Whether a host is among those the manifest names (`*.` a domain and what's under it). */
function reaches(net: string[], host: string) {
  return net.some((d) => (d.startsWith("*.") ? host === d.slice(2) || host.endsWith(d.slice(1)) : host === d));
}

function perform(start: Start, r: HttpRequest): HttpResponse {
  let host: string;
  try {
    host = new URL(r.url).hostname.toLowerCase();
  } catch {
    throw decline("refused", `not a URL: ${r.url}`);
  }
  if (!reaches(start.net, host)) throw decline("refused", `${host} is not among its hosts`);
  const x = new XMLHttpRequest();
  const method = r.method.toUpperCase();
  x.open(method, start.proxy ? `${start.proxy}?to=${encodeURIComponent(r.url)}` : r.url, false);
  x.responseType = "arraybuffer";
  x.timeout = r.timeoutMs ?? 30_000;
  if (start.proxy) x.setRequestHeader("X-Plugin-Headers", JSON.stringify(r.headers.map((h) => [h.name, h.value])));
  else for (const h of r.headers) x.setRequestHeader(h.name, h.value);
  try {
    x.send(r.body as XMLHttpRequestBodyInit | undefined);
  } catch (e) {
    throw decline((e as Error).name === "TimeoutError" ? "timeout" : "network", String(e));
  }
  const headers = x
    .getAllResponseHeaders()
    .trim()
    .split(/\r?\n/)
    .filter(Boolean)
    .map((l) => {
      const at = l.indexOf(":");
      return { name: l.slice(0, at), value: l.slice(at + 1).trim() };
    });
  return { status: x.status, headers, body: new Uint8Array(x.response as ArrayBuffer) };
}

/** The plugin API's imports, over what it was started with. */
function imports(start: Start) {
  const { data, rows, values } = start;
  const told = (m: unknown) => scope.postMessage(m);
  const set = (key: string, value: Uint8Array) => {
    data.set(key, value);
    told({ set: key, value });
  };
  const unset = (key: string) => {
    data.delete(key);
    told({ unset: key });
  };
  const answers = new Map<bigint, Answer>();
  let nextRequest = 1n;
  const timers = new Map<number, ReturnType<typeof setTimeout>>();
  const CACHE = "cache\0";
  return {
    ...wasi((words) => told({ log: words })),
    "nori:plugin/log": { write: (level: string, line: string) => told({ log: `${level}: ${line}` }) },
    "nori:plugin/store": { get: (k: string) => data.get(k), set, remove: unset, keys: (prefix: string) => [...data.keys()].filter((k) => k.startsWith(prefix) && !k.startsWith(CACHE)) },
    "nori:plugin/cache": { get: (k: string) => data.get(CACHE + k), put: (k: string, v: Uint8Array) => set(CACHE + k, v), remove: (k: string) => unset(CACHE + k), path: () => undefined },
    "nori:plugin/http": {
      fetch: (r: HttpRequest) => perform(start, r),
      // Done by the time `start` returns: a worker has one thread and this XHR blocks it.
      start: (r: HttpRequest) => {
        const id = nextRequest++;
        try {
          answers.set(id, { ok: perform(start, r) });
        } catch (e) {
          answers.set(id, { failed: (e as { payload: { kind: string; detail: string } }).payload });
        }
        return id;
      },
      wait: (ids: BigUint64Array) => ids[0],
      take: (id: bigint) => {
        const a = answers.get(id);
        answers.delete(id);
        if (!a) return undefined;
        return "ok" in a ? { tag: "ok", val: a.ok } : { tag: "err", val: a.failed };
      },
    },
    "nori:plugin/notices": { show: (tone: string, words: string, args: string[]) => told({ notice: { tone, words, args } }) },
    "nori:plugin/rows": {
      get: (id: string) => {
        const row = rows.find((r) => r.id === id);
        const v = row && valueOf(row, values);
        return row && v !== undefined ? witValue(row, v) : undefined;
      },
      setStatus: (row: string, words: string, args: string[]) => told({ status: { row, words, args } }),
      setShown: (row: string, shown: boolean, enabled: boolean) => told({ shown: { row, shown, enabled } }),
    },
    "nori:plugin/timers": {
      start: (id: number, ms: number, repeat: boolean) => {
        clearTimeout(timers.get(id));
        const fire = () => {
          if (repeat) timers.set(id, setTimeout(fire, ms));
          exportsOf?.hooks.on({ tag: "timer", val: id });
        };
        timers.set(id, setTimeout(fire, ms));
      },
      stop: (id: number) => clearTimeout(timers.get(id)),
    },
  };
}

type Exports = Record<"lifecycle" | "hooks" | "slots", Record<string, (...args: unknown[]) => unknown>>;
let exportsOf: Exports | undefined;

async function run(start: Start) {
  const url = URL.createObjectURL(new Blob([start.code.js], { type: "text/javascript" }));
  const glue = await import(/* @vite-ignore */ url);
  URL.revokeObjectURL(url);
  const modules = new Map(await Promise.all(Object.entries(start.code.cores).map(async ([name, bytes]) => [name, await WebAssembly.compile(bytes as BufferSource)] as const)));
  exportsOf = await glue.instantiate((name: string) => modules.get(name), imports(start), WebAssembly.instantiate);
}

let ready: Promise<void> | undefined;

scope.onmessage = async (e: MessageEvent) => {
  if (!ready) {
    ready = run(e.data as Start);
    ready.then(
      () => scope.postMessage({ started: true }),
      (err) => scope.postMessage({ failed: String(err) }),
    );
    return;
  }
  await ready;
  const { n, at, call, args } = e.data as Call;
  try {
    scope.postMessage({ n, value: exportsOf![at][call](...args) });
  } catch (err) {
    scope.postMessage({ n, error: String(err) });
  }
};
