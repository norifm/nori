// What installed plugins keep in IndexedDB: the list, each one's transpiled code, and its store.

import type { Code, Installed } from "./types";

const open = () =>
  new Promise<IDBDatabase>((resolve, reject) => {
    const r = indexedDB.open("nori-plugins", 1);
    r.onupgradeneeded = () => {
      for (const name of ["installed", "code", "data"]) r.result.createObjectStore(name);
    };
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });

const opened = open();

async function run<T>(store: string, mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await opened;
  return new Promise((resolve, reject) => {
    const r = f(db.transaction(store, mode).objectStore(store));
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

export const list = () => run<Installed[]>("installed", "readonly", (s) => s.getAll());
export const save = (i: Installed) => run("installed", "readwrite", (s) => s.put(i, i.plugin.id));
export const code = (id: string) => run<Code>("code", "readonly", (s) => s.get(id));
export const saveCode = (id: string, c: Code) => run("code", "readwrite", (s) => s.put(c, id));

/** `id`'s store, as its worker starts with it. */
export async function data(id: string): Promise<Map<string, Uint8Array>> {
  const range = IDBKeyRange.bound(`${id}\0`, `${id}\0￿`);
  const [keys, values] = await Promise.all([run("data", "readonly", (s) => s.getAllKeys(range)), run<Uint8Array[]>("data", "readonly", (s) => s.getAll(range))]);
  return new Map(keys.map((k, i) => [(k as string).slice(id.length + 1), values[i]]));
}

export const put = (id: string, key: string, value: Uint8Array) => run("data", "readwrite", (s) => s.put(value, `${id}\0${key}`));
export const drop = (id: string, key: string) => run("data", "readwrite", (s) => s.delete(`${id}\0${key}`));

/** Everything of `id`. */
export async function remove(id: string) {
  await Promise.all([run("installed", "readwrite", (s) => s.delete(id)), run("code", "readwrite", (s) => s.delete(id)), run("data", "readwrite", (s) => s.delete(IDBKeyRange.bound(`${id}\0`, `${id}\0￿`)))]);
}
