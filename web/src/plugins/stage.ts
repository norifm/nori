// A sound stage's code for the sound card's worklet: a worklet has no import() and no text codecs, so its
// glue goes in by addModule, after them, and registers itself for the worklet to find by id.

const CODECS = `
if (!globalThis.TextDecoder) globalThis.TextDecoder = class {
  constructor(label) { this.wide = label === "utf-16"; }
  decode(view) {
    if (!view) return "";
    const b = view instanceof ArrayBuffer ? new Uint8Array(view) : new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
    if (this.wide) return String.fromCharCode(...new Uint16Array(b.buffer, b.byteOffset, b.length >> 1));
    let s = "";
    for (let i = 0; i < b.length; ) {
      const c = b[i++];
      const n = c < 0x80 ? 0 : c < 0xe0 ? 1 : c < 0xf0 ? 2 : 3;
      let p = n === 0 ? c : c & (0x3f >> n);
      for (let k = 0; k < n; k++) p = (p << 6) | (b[i++] & 0x3f);
      s += String.fromCodePoint(p);
    }
    return s;
  }
};
if (!globalThis.TextEncoder) globalThis.TextEncoder = class {
  encode(s = "") {
    const out = [];
    for (const ch of s) {
      const p = ch.codePointAt(0);
      if (p < 0x80) out.push(p);
      else if (p < 0x800) out.push(0xc0 | (p >> 6), 0x80 | (p & 0x3f));
      else if (p < 0x10000) out.push(0xe0 | (p >> 12), 0x80 | ((p >> 6) & 0x3f), 0x80 | (p & 0x3f));
      else out.push(0xf0 | (p >> 18), 0x80 | ((p >> 12) & 0x3f), 0x80 | ((p >> 6) & 0x3f), 0x80 | (p & 0x3f));
    }
    return Uint8Array.from(out);
  }
  encodeInto(s, into) {
    const b = this.encode(s);
    into.set(b.subarray(0, into.length));
    return { read: s.length, written: Math.min(b.length, into.length) };
  }
};
`;

/** The module to add for plugin `id`'s stage, from its glue. */
export function stageModule(id: string, js: string): string {
  const source = `${CODECS}\n${js};\n(globalThis.noriStages ??= {})[${JSON.stringify(id)}] = instantiate;\n`;
  return URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
}
