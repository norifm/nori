// The page: draws what the worker answers and asks it for more. The core and the engine live in the
// worker; the sound card is an AudioWorklet the worker feeds directly.

type Album = { id: string; name: string; artist: string; year: number; cover: string | null };
type Song = { id: string; title: string; artist: string; duration: number; suffix: string };

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const started = performance.now();
let audio: AudioContext | undefined;

/** Timings and what was heard, for a headless browser checking the page. */
const facts: Record<string, unknown> = { shellMs: Math.round(started) };
(window as unknown as { nori: unknown }).nori = facts;

function say(text: string) {
  $("status").textContent = text;
}

worker.onmessage = (e: MessageEvent) => {
  const m = e.data;
  if (m.ok === false) {
    say(`${m.op}: ${m.error}`);
    facts.error = `${m.op}: ${m.error}`;
    return;
  }
  switch (m.op) {
    case "ready":
      facts.coreMs = m.ms;
      facts.stored = m.stored;
      facts.coreReadyAt = Math.round(performance.now());
      say(`core ready in ${m.ms} ms`);
      break;
    case "login":
      facts.loginMs = m.ms;
      say("signed in");
      $("login").hidden = true;
      worker.postMessage({ op: "albums", size: 24 });
      break;
    case "albums":
      facts.albums = m.list.length;
      facts.albumsMs = m.ms;
      albums(m.list);
      break;
    case "album":
      songs(m.songs);
      break;
    case "playing":
      facts.fetchAndStartMs = m.ms;
      say(`playing (fetched and started in ${m.ms} ms)`);
      break;
    case "heard": {
      const ctx = audio!;
      facts.heard = { seconds: m.played / ctx.sampleRate, dry: m.dry, peak: m.peak, status: m.status };
      const s = m.status;
      $("now").textContent = `${s.state} ${(s.positionMs / 1000).toFixed(1)} s, heard ${(m.played / ctx.sampleRate).toFixed(1)} s, peak ${m.peak.toFixed(2)}, dry ${m.dry}`;
      break;
    }
  }
};

function albums(list: Album[]) {
  const grid = $("albums");
  grid.replaceChildren(
    ...list.map((a) => {
      const card = document.createElement("button");
      card.className = "album";
      const img = document.createElement("img");
      img.alt = "";
      img.loading = "lazy";
      if (a.cover) img.src = a.cover;
      const name = document.createElement("span");
      name.textContent = a.name;
      const artist = document.createElement("small");
      artist.textContent = a.artist;
      card.append(img, name, artist);
      card.onclick = () => worker.postMessage({ op: "album", id: a.id });
      return card;
    }),
  );
}

function songs(list: Song[]) {
  const ol = $("songs");
  ol.replaceChildren(
    ...list.map((s) => {
      const li = document.createElement("li");
      const b = document.createElement("button");
      b.textContent = `${s.title} - ${s.artist} (${Math.floor(s.duration / 60)}:${String(s.duration % 60).padStart(2, "0")}, ${s.suffix})`;
      b.onclick = () => play(s.id);
      li.append(b);
      return li;
    }),
  );
}

/** The first Play makes the sound card (a user gesture is needed) and joins it to the worker. */
async function play(id: string) {
  if (!audio) {
    audio = new AudioContext({ latencyHint: "playback" });
    await audio.audioWorklet.addModule(new URL("./worklet.js", import.meta.url));
    const node = new AudioWorkletNode(audio, "nori-out", { outputChannelCount: [2] });
    node.connect(audio.destination);
    const line = new MessageChannel();
    node.port.postMessage(line.port1, [line.port1]);
    worker.postMessage({ op: "play", id, rate: audio.sampleRate, port: line.port2 }, [line.port2]);
  } else {
    worker.postMessage({ op: "play", id, rate: audio.sampleRate });
  }
  await audio.resume();
  facts.rate = audio.sampleRate;
}

$("login").onsubmit = (e) => {
  e.preventDefault();
  const form = new FormData(e.target as HTMLFormElement);
  worker.postMessage({ op: "login", url: location.origin, user: form.get("user"), password: form.get("password") });
};
$("pause").onclick = () => worker.postMessage({ op: "pause" });
(window as unknown as { play: unknown }).play = play;
