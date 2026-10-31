// Installing a plugin: its package opened, checked, read by the core and transpiled to JavaScript, which is
// the one way a browser runs a component. Loaded only when one is installed: the generator is 4 MB.

import { $init, generate } from "@bytecodealliance/jco-transpile/component";
import { ask } from "../rpc";
import type { Code, Read, WebPlugin } from "./types";

/** A package is a zip; these are the files of it nori uses. */
const WANTED = ["plugin.wasm", "strings/en.toml", "icon.png", "README.md"];

async function inflate(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

/** The wanted files of the zip in `zip`, by its central directory. */
async function unzip(zip: Uint8Array): Promise<Map<string, Uint8Array>> {
  const view = new DataView(zip.buffer, zip.byteOffset, zip.byteLength);
  let end = zip.length - 22;
  while (end >= 0 && view.getUint32(end, true) !== 0x06054b50) end--;
  if (end < 0) throw new Error("not a zip");
  const files = new Map<string, Uint8Array>();
  let at = view.getUint32(end + 16, true);
  for (let n = view.getUint16(end + 10, true); n > 0; n--) {
    const [method, packed] = [view.getUint16(at + 10, true), view.getUint32(at + 20, true)];
    const [nameLength, extra, comment] = [view.getUint16(at + 28, true), view.getUint16(at + 30, true), view.getUint16(at + 32, true)];
    const name = new TextDecoder().decode(zip.subarray(at + 46, at + 46 + nameLength));
    if (WANTED.includes(name)) {
      const local = view.getUint32(at + 42, true);
      const start = local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true);
      const body = zip.subarray(start, start + packed);
      files.set(name, method === 0 ? body : await inflate(body));
    }
    at += 46 + nameLength + extra + comment;
  }
  return files;
}

async function sha256(bytes: Uint8Array) {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export async function transpile(wasm: Uint8Array): Promise<Code> {
  await $init;
  const made = generate(wasm, { name: "plugin", instantiation: { tag: "async" }, noTypescript: true, noNodejsCompat: true, base64Cutoff: 0, tlaCompat: false });
  const cores: Record<string, Uint8Array> = {};
  let js = "";
  for (const [name, bytes] of made.files) {
    if (name.endsWith(".js")) js = new TextDecoder().decode(bytes);
    else if (name.endsWith(".wasm")) cores[name] = bytes;
  }
  return { js, cores };
}

/** What a package says of itself, for its sheet; refused when it isn't what the repo said (`sha256`). */
export async function read(pack: Uint8Array, proxied: boolean, want?: string): Promise<Read> {
  if (want && (await sha256(pack)) !== want) throw new Error("the package isn't what the repo lists");
  const files = await unzip(pack);
  const wasm = files.get("plugin.wasm");
  if (!wasm) throw new Error("no plugin.wasm in the package");
  const strings = new TextDecoder().decode(files.get("strings/en.toml") ?? new Uint8Array());
  const plugin = JSON.parse(await ask<string>("plugin_read", wasm, strings, proxied)) as WebPlugin;
  const png = files.get("icon.png");
  const icon = png && `data:image/png;base64,${btoa(String.fromCharCode(...png))}`;
  const readme = files.get("README.md");
  return { plugin, wasm, icon, readme: readme && new TextDecoder().decode(readme) };
}
