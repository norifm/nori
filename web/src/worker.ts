// The core in a Dedicated Worker: answers the page's calls (`{ id, call, args }` -> `{ id, value }` or
// `{ id, error }`), hands on the session's reports (`{ said }`), and feeds the AudioWorklet over its own
// port. Which module it runs (core or player) is the page's choice, in the worker's URL.

type Module = {
  default: (init?: { module_or_path: WebAssembly.Module }) => Promise<unknown>;
  Nori: { open(tell: (said: string) => void, wake: () => void): Promise<Record<string, (...a: unknown[]) => unknown>> };
};

const scope = self as unknown as Worker;
/** Interleaved frames pulled per message to the worklet. */
const CHUNK = 4096;

let nori: Record<string, (...a: unknown[]) => unknown> | undefined;
let worklet: MessagePort | undefined;
let timer: number | undefined;
let woken = false;
/** What the worklet was last told: the player holds (paused or stopped). */
let held = true;
// The worker's name says which module it runs (its address is the same for both, so it can be kept).
const player = self.name === "player";

/** The engine asked for a turn (a command, or a song's bytes arrived): one turn soon, however many asks. */
function wake() {
  if (woken) return;
  woken = true;
  queueMicrotask(() => {
    woken = false;
    pump();
  });
}

function tell(said: string) {
  scope.postMessage({ said: JSON.parse(said) });
}

/** A module's glue in dist/pkg, from this worker's place in dist/assets. */
function pkg(name: string): string {
  return new URL(`../pkg/${name}.js`, self.location.href).href;
}

/** The player module as the page compiled it while it downloaded, and its build's name. */
type Handed = { module: WebAssembly.Module; file: string };
let hand: (h: Handed) => void = () => {};
const handed = new Promise<Handed>((r) => (hand = r));

const ready = (async () => {
  const t0 = performance.now();
  // The browser's own decoders, asked before the player starts: it decodes with them what they can.
  // dist/decoder.js, the very module the player's glue imports (so the codecs probed are its).
  const codecs = player ? await ((await import(/* @vite-ignore */ new URL("../decoder.js", self.location.href).href)) as typeof import("./decoder")).probe(wake) : [];
  const given = player ? await handed : undefined;
  const m = (await import(/* @vite-ignore */ pkg(given?.file ?? "core"))) as Module;
  await m.default(given && { module_or_path: given.module });
  nori = await m.Nori.open(tell, wake);
  scope.postMessage({ ready: { ms: Math.round(performance.now() - t0), stored: nori.stored, player, codecs } });
})();

/** The pull buffer, kept between passes; a transferred one is detached (length 0) and so replaced. */
let out = new Float32Array(0);

/** Runs the engine, tops the worklet up, and comes back when the engine asks to. */
function pump() {
  if (!nori || !player) return;
  if (timer !== undefined) clearTimeout(timer);
  timer = undefined;
  let next = nori.turn() as number;
  order();
  const channels = nori.channels() as number;
  while (worklet && channels > 0) {
    if (out.length !== CHUNK * channels) out = new Float32Array(CHUNK * channels);
    const frames = nori.pull(out) as number;
    if (frames === 0) break;
    const pcm = frames === CHUNK ? out : out.slice(0, frames * channels);
    worklet.postMessage({ pcm, channels }, [pcm.buffer]);
    next = nori.turn() as number;
    order();
  }
  const nowHeld = !(nori.playing() as boolean);
  if (worklet && nowHeld !== held) {
    held = nowHeld;
    worklet.postMessage({ held });
  }
  // Fed and idle: the worklet's next report brings the next turn.
  if (next >= 0) timer = setTimeout(pump, Math.max(next, 1)) as unknown as number;
}

/** Hands the worklet what the engine's output told it. */
function order() {
  if (!worklet) return;
  const orders = nori!.orders() as string;
  if (orders) for (const o of JSON.parse(orders)) worklet.postMessage(o);
}

/** The sound card's port, rate and latency to the ear (s), from the page on the first Play. */
function audio(port: MessagePort, rate: number, ear: number) {
  nori!.set_card(rate, ear * 1000);
  worklet = port;
  worklet.onmessage = (w: MessageEvent) => {
    const { played, flushed } = w.data;
    if (flushed) nori!.flushed(played, flushed.dropped, flushed.flushes);
    else nori!.played(played);
    scope.postMessage({ heard: w.data });
    pump();
  };
}

scope.onmessage = async (e: MessageEvent) => {
  if (e.data.handed) return hand(e.data.handed);
  await ready;
  const { id, call, args } = e.data as { id: number; call: string; args: unknown[] };
  try {
    let value: unknown;
    if (call === "audio") audio(args[0] as MessagePort, args[1] as number, args[2] as number);
    else {
      const f = nori![call];
      if (typeof f !== "function") throw new Error(`no call ${call}`);
      value = await f.apply(nori, args);
    }
    scope.postMessage({ id, value });
  } catch (err) {
    scope.postMessage({ id, error: String(err) });
    // A panic in Rust aborts the module: nothing it holds can be used any more.
    if (err instanceof WebAssembly.RuntimeError) scope.postMessage({ crashed: String(err) });
  }
  pump();
};
