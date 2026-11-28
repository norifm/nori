// A local profile's folder in this browser. It is picked (through the File System Access API where the
// browser has it, else an `<input webkitdirectory>`), walked into its files, and handed to the worker,
// which reads them (crates/web files.rs). A picked folder's handle is kept by its profile's id, so a later
// visit reads it again once the browser allows it; a folder from the input is picked again each visit.

import { ask, handFolder } from "../rpc";
import { library } from "./library.svelte";

/** One file of the folder as the worker takes it: `path` from the folder's name on. */
type Found = { path: string; size: number; modified: number; file: File };

/** What of the File System Access API is used here (TypeScript's DOM types lack most of it). */
type Permission = { mode: "read" };
type FileHandle = { kind: "file"; name: string; getFile(): Promise<File> };
type DirectoryHandle = {
  kind: "directory";
  name: string;
  entries(): AsyncIterable<[string, FileHandle | DirectoryHandle]>;
  queryPermission(p: Permission): Promise<PermissionState>;
  requestPermission(p: Permission): Promise<PermissionState>;
};
type Picker = (o: { id: string; mode: "read" }) => Promise<DirectoryHandle>;

/** A folder picked: its name, its files, and its handle where the browser gives one. */
export type Picked = { name: string; files: Found[]; handle?: DirectoryHandle };

const picker = (window as unknown as { showDirectoryPicker?: Picker }).showDirectoryPicker;

class Folder {
  /** The open profile reads a folder the worker hasn't been handed this visit: its songs play once it is
   * allowed or picked again (`allow`). */
  waits = $state(false);
  /** That folder's name, where its handle is kept. */
  name = $state("");
  /** Its handle, kept from an earlier visit. */
  private handle: DirectoryHandle | undefined;
  /** The profile it is the folder of. */
  private id = "";

  /** A profile opened: a local one's folder is read again at once where the browser still allows it, and
   * otherwise waits for `allow`. */
  async opened() {
    const id = await ask<string | null>("local_waits");
    this.waits = false;
    if (!id) return;
    this.id = id;
    this.handle = await kept(id);
    this.name = this.handle?.name ?? "";
    if (this.handle && (await this.handle.queryPermission({ mode: "read" })) === "granted") await this.read({ name: this.handle.name, files: await walk(this.handle), handle: this.handle });
    else this.waits = true;
  }

  /** On a press: the browser asked to allow the kept folder again, or the folder picked anew. */
  async allow() {
    const again = this.handle && (await this.handle.requestPermission({ mode: "read" })) === "granted";
    const picked = again && this.handle ? { name: this.handle.name, files: await walk(this.handle), handle: this.handle } : await pick();
    if (picked) await this.read(picked);
  }

  /** Adds a profile of `picked`, open from then on (or attached as a source of `forProfile`). */
  async add(picked: Picked, forProfile?: string) {
    this.id = await ask<string>("new_profile_id");
    if (forProfile) await ask("add_local_source", forProfile, this.id, picked.name);
    else await ask("add_local", this.id, picked.name);
    await this.read(picked);
  }

  /** Keeps `picked`'s handle, hands its files over and reads what changed into the library. */
  private async read(picked: Picked) {
    if (picked.handle) await keep(this.id, picked.handle);
    this.handle = picked.handle ?? this.handle;
    await handFolder(this.id, picked.files);
    this.waits = false;
    this.name = picked.name;
    library.changed();
    await ask("sync_index").catch(() => {});
    library.changed();
  }
}

export const folder = new Folder();

/** Asks for a folder, on a press; null when none was picked. */
export async function pick(): Promise<Picked | null> {
  if (picker) {
    try {
      const handle = await picker({ id: "nori-music", mode: "read" });
      return { name: handle.name, files: await walk(handle), handle };
    } catch (e) {
      if ((e as Error).name === "AbortError") return null;
      throw e;
    }
  }
  return new Promise((done) => {
    const input = document.createElement("input");
    input.type = "file";
    input.webkitdirectory = true;
    input.multiple = true;
    input.addEventListener("cancel", () => done(null));
    input.addEventListener("change", () => {
      const files = Array.from(input.files ?? []).map((file) => ({ path: file.webkitRelativePath, size: file.size, modified: file.lastModified, file }));
      done(files.length ? { name: files[0].path.split("/")[0], files } : null);
    });
    input.click();
  });
}

/** Every file under `dir`, hidden ones left out; the worker keeps those it reads. */
async function walk(dir: DirectoryHandle, at = dir.name, out: Found[] = []): Promise<Found[]> {
  for await (const [name, h] of dir.entries()) {
    if (name.startsWith(".")) continue;
    const path = `${at}/${name}`;
    if (h.kind === "directory") await walk(h, path, out);
    else {
      const file = await h.getFile();
      out.push({ path, size: file.size, modified: file.lastModified, file });
    }
  }
  return out;
}

const DB = "nori-folders";
const HANDLES = "handles";

function db(): Promise<IDBDatabase> {
  return new Promise((done, failed) => {
    const open = indexedDB.open(DB, 1);
    open.onupgradeneeded = () => open.result.createObjectStore(HANDLES);
    open.onsuccess = () => done(open.result);
    open.onerror = () => failed(open.error);
  });
}

/** The folder handle kept for profile `id`. */
async function kept(id: string): Promise<DirectoryHandle | undefined> {
  const d = await db();
  return new Promise((done) => {
    const got = d.transaction(HANDLES).objectStore(HANDLES).get(id);
    got.onsuccess = () => done(got.result as DirectoryHandle | undefined);
    got.onerror = () => done(undefined);
  });
}

async function keep(id: string, handle: DirectoryHandle) {
  const d = await db();
  await new Promise<void>((done, failed) => {
    const t = d.transaction(HANDLES, "readwrite");
    t.objectStore(HANDLES).put(handle, id);
    t.oncomplete = () => done();
    t.onerror = () => failed(t.error);
  });
}
