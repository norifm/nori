// The core in a Dedicated Worker: nori-core and the engine (core.wasm), answering the page's messages,
// and feeding the AudioWorklet over its own port.

import init, { Nori } from "./pkg/core.js";

const scope = self as unknown as Worker;
/** Interleaved frames pulled per message to the worklet. */
const CHUNK = 4096;
/** How far ahead of the ear the worklet is kept fed, s. */
const AHEAD_S = 0.5;

let nori: Nori | undefined;
let worklet: MessagePort | undefined;
let rate = 48000;
/** Frames sent to the worklet and frames it played. */
let sent = 0;
let played = 0;
let timer: number | undefined;

const ready = (async () => {
  const t0 = performance.now();
  await init();
  nori = await Nori.open();
  scope.postMessage({ op: "ready", ms: Math.round(performance.now() - t0), stored: nori.stored });
})();

/** Runs the engine, tops the worklet up, and comes back when the engine asks to. */
function pump() {
  if (!nori) return;
  if (timer !== undefined) clearTimeout(timer);
  timer = undefined;
  let wake = nori.turn();
  const channels = nori.channels();
  while (worklet && channels > 0 && sent - played < AHEAD_S * rate) {
    const out = new Float32Array(CHUNK * channels);
    const frames = nori.pull(out);
    if (frames === 0) break;
    const pcm = frames === CHUNK ? out : out.slice(0, frames * channels);
    worklet.postMessage({ pcm, channels }, [pcm.buffer]);
    sent += frames;
    wake = nori.turn();
  }
  // Fed and idle: the worklet's next report brings the next turn.
  if (wake >= 0) timer = setTimeout(pump, Math.max(wake, 1)) as unknown as number;
}

scope.onmessage = async (e: MessageEvent) => {
  await ready;
  const m = e.data;
  const n = nori!;
  try {
    switch (m.op) {
      case "login": {
        const t0 = performance.now();
        await n.login(m.url, m.user, m.password);
        scope.postMessage({ op: "login", ok: true, ms: Math.round(performance.now() - t0) });
        break;
      }
      case "albums": {
        const t0 = performance.now();
        const list = JSON.parse(await n.albums(m.size));
        scope.postMessage({ op: "albums", list, ms: Math.round(performance.now() - t0) });
        break;
      }
      case "album":
        scope.postMessage({ op: "album", id: m.id, songs: JSON.parse(await n.album(m.id)) });
        break;
      case "play": {
        if (m.port) {
          worklet = m.port as MessagePort;
          worklet.onmessage = (w: MessageEvent) => {
            played = w.data.played;
            scope.postMessage({ op: "heard", ...w.data, status: JSON.parse(n.status()) });
            pump();
          };
        }
        rate = m.rate;
        const t0 = performance.now();
        await n.play(m.id, m.rate);
        scope.postMessage({ op: "playing", id: m.id, ms: Math.round(performance.now() - t0) });
        pump();
        break;
      }
      case "pause":
        n.pause();
        pump();
        break;
    }
  } catch (err) {
    scope.postMessage({ op: m.op, ok: false, error: String(err) });
  }
};
