// The page's line to the worker: calls answered by promise, the session's reports to whoever listens.
// The worker first runs the core module (sign-in, the library), so the first page comes quickly; once it
// shows, it is replaced in the background by one running the player module, which opens the same
// databases. Only one worker may hold them, so only one tab runs nori at a time (a Web Lock), and a
// worker that crashed is started again over the saved session.


export type Said = { kind: string; [k: string]: unknown };
export type Heard = { played: number; dry: number; dryAt: number[]; peak: number; silences: [number, number][]; lastLoud: number; soundAt: number; freshAt: number };
/** A worker's build: the core, or the player with its compiled module. */
type Build = { kind: "core" } | { kind: "player"; module: WebAssembly.Module; file: string };

/** Timings and what was heard, for a headless browser checking the page. */
export const facts: Record<string, unknown> = { shellMs: Math.round(performance.now()) };
(window as unknown as { nori: unknown }).nori = facts;

/** Where this tab's line to the worker is. Calls wait while a worker is on its way (booting, switching
 * to the player, restarting after a crash) and fail once none will come (another tab has nori). */
type Phase = "booting" | "lockedOut" | "core" | "switching" | "player" | "restarting" | "movedAway";
let phase: Phase = "booting";

let worker: Worker;
let build: Build = { kind: "core" };
let calls = 0;
const waiting = new Map<number, { done: (v: unknown) => void; failed: (e: Error) => void }>();
let opened!: () => void;
/** Settles when calls may go on; pending while a worker is on its way. */
let gate = new Promise<void>((r) => (opened = r));
/** The worker answering (or on its way) runs the player module. */
export let player = false;

/** Enters `p`: calls wait while a worker is on its way, and go on (or fail) once it is up (or none
 * will come). */
function enter(p: Phase) {
  const waits = p === "switching" || p === "restarting";
  if (waits && !(phase === "switching" || phase === "restarting")) gate = new Promise((r) => (opened = r));
  phase = p;
  if (!waits) opened();
}

/** The calls not answered fail with `why`. */
function dropWaiting(why: string) {
  for (const [, c] of waiting) c.failed(new Error(why));
  waiting.clear();
  quiet.splice(0).forEach((r) => r());
}

const listeners: ((s: Said) => void)[] = [];
const up: (() => void)[] = [];
const restarted: (() => void)[] = [];

export function onSaid(f: (s: Said) => void) {
  listeners.push(f);
}

/** `f` runs once the player worker answers (the first time, and after a crash). */
export function onPlayerUp(f: () => void) {
  up.push(f);
}

/** `f` runs once a crashed worker is up again over the saved session. */
export function onRestart(f: () => void) {
  restarted.push(f);
}

/** Who waits for every call to be answered. */
const quiet: (() => void)[] = [];

function settled(): Promise<void> {
  return waiting.size === 0 ? Promise.resolve() : new Promise((r) => quiet.push(r));
}

/** Starts a worker over `b`; resolves once it is up. */
function start(b: Build): Promise<void> {
  const w = new Worker(new URL("./worker.ts", import.meta.url), { type: "module", name: b.kind });
  worker = w;
  build = b;
  if (b.kind === "player") w.postMessage({ handed: { module: b.module, file: b.file } });
  let up = false;
  return new Promise((ready, failed) => {
    w.onerror = (e) => {
      if (!up) return failed(new Error(e.message));
      e.preventDefault();
      crashed(w, e.message);
    };
    w.onmessage = (e: MessageEvent) => {
      const m = e.data;
      if (m.ready) {
        facts[`${b.kind}Ms`] = m.ready.ms;
        facts.stored = m.ready.stored;
        if (m.ready.codecs) facts.codecs = m.ready.codecs;
        up = true;
        ready();
      } else if (m.crashed) {
        crashed(w, m.crashed);
      } else if (m.said) {
        listeners.forEach((l) => l(m.said));
      } else if (m.id) {
        const c = waiting.get(m.id);
        waiting.delete(m.id);
        if (waiting.size === 0) quiet.splice(0).forEach((r) => r());
        if (m.error !== undefined) {
          facts.error = m.error;
          c?.failed(new Error(m.error));
        } else c?.done(m.value);
      }
    };
  });
}

function send<T>(call: string, args: unknown[]): Promise<T> {
  const id = ++calls;
  return new Promise((done, failed) => {
    waiting.set(id, { done: done as (v: unknown) => void, failed });
    worker.postMessage({ id, call, args });
  });
}

/** Calls the worker's `call` with `args`; while the worker is being replaced, once it is. Fails when
 * another tab has nori. */
export async function ask<T = unknown>(call: string, ...args: unknown[]): Promise<T> {
  await gate;
  if (phase === "lockedOut" || phase === "movedAway") throw new Error("nori is open in another tab");
  return send<T>(call, args);
}

/** Reads a page (`{ kind, ... }`) as its JSON. */
export async function read<T = Record<string, unknown>>(ask_: Record<string, unknown>): Promise<T> {
  return JSON.parse(await ask<string>("read", JSON.stringify(ask_)));
}

/** Replaces the worker with one over `b` while calls wait: the old one answers what it was asked and
 * goes, the new one opens the profile signed in last. */
async function replace(b: Build, answered: boolean) {
  while (phase === "booting" || phase === "switching" || phase === "restarting") await gate;
  if (phase !== "core" && phase !== "player") return;
  enter(answered ? "switching" : "restarting");
  if (answered) await settled();
  else dropWaiting("the worker stopped");
  worker.terminate();
  await start(b);
  player = b.kind === "player";
  await send("resume", []);
  if (sound && player) {
    const { port, rate, ear } = sound();
    worker.postMessage({ id: ++calls, call: "audio", args: [port, rate, ear] }, [port]);
  }
  enter(b.kind);
  if (player) {
    up.forEach((f) => f());
    // Here, not before: this worker stays, where a call on the core's would hold up the move or be cut.
    send("sync_index", []).catch(() => {});
  }
}

let restart: Promise<void> | undefined;

/** The worker `w` crashed (a panic in Rust aborts its module): another starts over the saved session. */
function crashed(w: Worker, why: string) {
  if (w !== worker || restart) return;
  facts.crashes = ((facts.crashes as number) ?? 0) + 1;
  facts.crashed = why;
  restart = replace(build, false).then(() => {
    restart = undefined;
    restarted.forEach((f) => f());
  });
}

let switching: Promise<void> | undefined;

/** Moves to the player module, once. Its build is downloaded and compiled here while the core worker
 * goes on answering; then the core worker makes way for one over the compiled module. Started in the
 * background once the first page shows; a Play before then waits for the same move. */
export function playerUp(): Promise<void> {
  if (switching) return switching;
  switching = (async () => {
    const t0 = performance.now();
    // dist/decoder.js, built on its own for the player's glue; asked here which build decodes what.
    const { playerFile, probe } = (await import(/* @vite-ignore */ new URL("decoder.js", document.baseURI).href)) as typeof import("./decoder");
    const file = playerFile(await probe(() => {}));
    const module = await WebAssembly.compileStreaming(fetch(new URL(`pkg/${file}.wasm`, document.baseURI)));
    facts.playerCompiledMs = Math.round(performance.now() - t0);
    const t1 = performance.now();
    await replace({ kind: "player", module, file }, true);
    facts.playerSwitchMs = Math.round(performance.now() - t1);
    facts.playerReadyAt = Math.round(performance.now());
  })();
  return switching;
}

/** A line to the sound card: its port for the player worker, the card's rate, and its latency to the ear, s. */
type Line = { port: MessagePort; rate: number; ear: number };
/** Makes a new line to the sound card. */
let sound: (() => Line) | undefined;

/** Joins the sound card to the player worker, now and after every restart. */
export async function soundTo(line: () => Line) {
  await playerUp();
  sound = line;
  const { port, rate, ear } = line();
  worker.postMessage({ id: ++calls, call: "audio", args: [port, rate, ear] }, [port]);
}

/** Starts the player module when the browser is idle after the first page, unless data is to be saved
 * (then it starts on the first Play). */
export function playerSoon() {
  const save = (navigator as Navigator & { connection?: { saveData?: boolean } }).connection?.saveData;
  facts.playerEarly = !save;
  if (save) return;
  const go = () => void playerUp();
  if ("requestIdleCallback" in window) requestIdleCallback(go, { timeout: 2000 });
  else setTimeout(go, 200);
}

// ---- one tab ----

const LOCK = "nori-databases";
const tabs = new BroadcastChannel("nori-tabs");
/** Ends this tab's hold on the databases. */
let release: (() => void) | undefined;
const movedAway: (() => void)[] = [];

/** `f` runs when another tab took nori over: this one's worker has gone. */
export function onMovedAway(f: () => void) {
  movedAway.push(f);
}

/** Holds the databases for this tab until another asks for them (`take`). */
function hold(): Promise<void> {
  return new Promise((held) => {
    void navigator.locks.request(LOCK, () => {
      held();
      return new Promise<void>((r) => (release = r));
    });
  });
}

tabs.onmessage = async (e: MessageEvent) => {
  if (e.data !== "take" || !release) return;
  // Another tab wants nori: the session is saved and closed, the worker goes, the lock is let go.
  await ask("close").catch(() => {});
  worker.terminate();
  player = false;
  enter("movedAway");
  dropWaiting("nori moved to another tab");
  const r = release;
  release = undefined;
  r();
  movedAway.forEach((f) => f());
};

/** Starts nori in this tab: false when another tab has it (then nothing starts). */
export async function boot(): Promise<boolean> {
  const free = await navigator.locks.query().then((s) => !(s.held ?? []).some((l) => l.name === LOCK));
  if (!free) {
    enter("lockedOut");
    return false;
  }
  await hold();
  await start({ kind: "core" });
  enter("core");
  return true;
}

/** Takes nori over from the tab that has it: once that one let go, this page starts again. */
export async function takeOver(): Promise<void> {
  tabs.postMessage("take");
  await navigator.locks.request(LOCK, () => {});
  location.reload();
}
