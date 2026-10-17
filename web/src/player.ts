// The player: the mini bar, the Now Playing sheet with its lyrics and queue, and the sound card. What
// plays and how is the session's (in the worker); this draws it and asks.

import { ask, facts, onHeard, onRestart, onSaid, player, soundTo } from "./rpc.js";
import { cover, h, icon, iconButton } from "./dom.js";
import { panel } from "./jam.js";
import { heart } from "./menu.js";
import { W } from "./words.js";

export type Song = { id: string; title: string; artist: string; album: string; albumId?: string; artistId?: string; duration: number; cover?: string; suffix?: string };
type Now = {
  state: string;
  index: number | null;
  id: string | null;
  positionMs: number;
  pace: number;
  mixing: boolean;
  song: Song | null;
  queue: Song[];
  current: number | null;
  shuffle: boolean;
  repeat: number;
  underruns: number;
};
type Lyrics = { synced: boolean; lines: { text: string; startMs: number }[] };

let audio: AudioContext | undefined;
let now: Now | undefined;
/** When `now` was read, on the page's clock. */
let readAt = 0;
let lyrics: { song: string; lyrics: Lyrics } | undefined;
let sheetOpen = false;
let tab: "lyrics" | "queue" | "jam" = "lyrics";
let toastTimer: number | undefined;

const mini = () => document.getElementById("mini")!;
const sheet = () => document.getElementById("sheet")!;

/** The sound card, made on the first Play (browsers let audio start only from a gesture): an
 * AudioWorklet the player worker feeds over its own port. */
export function soundCard(): Promise<void> {
  if (audio) {
    void audio.resume();
    return Promise.resolve();
  }
  audio = new AudioContext({ latencyHint: "playback" });
  void audio.resume();
  const ctx = audio;
  return (async () => {
    await ctx.audioWorklet.addModule(new URL("./worklet.js", import.meta.url));
    const node = new AudioWorkletNode(ctx, "nori-out", { outputChannelCount: [2] });
    node.connect(ctx.destination);
    facts.rate = ctx.sampleRate;
    // The sound card's clock, which the worklet's times are on.
    Object.defineProperty(facts, "audioTime", { get: () => ctx.currentTime, enumerable: true });
    if (!player) say(W.loadingPlayer);
    // A new line each time the player worker starts (again, after a crash).
    await soundTo(() => {
      const line = new MessageChannel();
      node.port.postMessage(line.port1, [line.port1]);
      return { port: line.port2, rate: ctx.sampleRate, ear: ctx.baseLatency + (ctx.outputLatency || 0) };
    });
    say("");
  })();
}

/** A line of news over the page, for a few seconds. */
function say(text: string) {
  const s = document.getElementById("toast");
  if (!s) return;
  s.textContent = text;
  s.hidden = !text;
  clearTimeout(toastTimer);
  if (text) toastTimer = setTimeout(() => (s.hidden = true), 4000) as unknown as number;
}

/** Plays `songs` from `start`; `kind` and `id` name the page they are the list of. Call from a click. */
export async function play(songs: Song[], start: number, shuffle = false, kind = "", id = "") {
  await soundCard();
  await ask("play", JSON.stringify(songs), start, shuffle, kind, id);
  await refresh();
}

export async function enqueue(songs: Song[], next: boolean) {
  await soundCard();
  await ask("enqueue", JSON.stringify(songs), next);
  await refresh();
}

async function control(call: string, ...args: unknown[]) {
  if (!player) return;
  await ask(call, ...args);
  await refresh();
}

/** Reads what plays and draws it. */
/** What the mini bar and the sheet show, to draw them again only when it changes (a button drawn anew
 * under a finger loses its press). */
let shown = "";
/** The jam as its panel shows it. */
let jamShown = "";

async function refresh() {
  if (!player) return;
  const was = now?.id;
  now = JSON.parse(await ask<string>("now"));
  readAt = performance.now();
  if (now && now.id !== was) songChanged();
  const n = now;
  const key = JSON.stringify(n && [n.id, n.state, n.mixing, n.shuffle, n.repeat, n.current, n.queue.map((q) => q.id), (n.song as Song & { starred?: boolean } | null)?.starred]);
  if (key !== shown) {
    shown = key;
    drawMini();
    if (sheetOpen) drawSheet();
  }
  // A seek or a skip shows at once, playing or not.
  if (sheetOpen) {
    drawPlace();
    tick();
  }
}

/** The position now, run on from the last reading while playing. */
function position(): number {
  if (!now) return 0;
  const run = now.state === "Playing" ? (performance.now() - readAt) * (now.pace || 1) : 0;
  return Math.min(now.positionMs + run, (now.song?.duration ?? 0) * 1000 || Infinity);
}

function songChanged() {
  lyrics = undefined;
  const s = now?.song;
  if (s && "mediaSession" in navigator) {
    navigator.mediaSession.metadata = new MediaMetadata({ title: s.title, artist: s.artist, album: s.album, artwork: s.cover ? [{ src: s.cover, sizes: "300x300" }] : [] });
  }
  if (s && sheetOpen && tab === "lyrics") void ask("lyrics", s.id);
}

function drawMini() {
  const s = now?.song;
  const m = mini();
  m.hidden = !s;
  if (!s) return;
  const playing = now!.state === "Playing";
  m.replaceChildren(
    cover(s.cover, "cover small"),
    h("div", { class: "mini-text" }, h("span", { class: "title" }, s.title), h("span", { class: "sub" }, s.artist)),
    iconButton(playing ? "pause" : "play", playing ? W.pause : W.play, () => control("toggle"), "plain"),
    iconButton("next", W.next, () => control("next"), "plain"),
  );
  m.onclick = () => openSheet();
}

export function openSheet() {
  sheetOpen = true;
  sheet().hidden = false;
  document.body.classList.add("sheet-open");
  drawSheet();
  if (now?.song && tab === "lyrics" && !lyrics) void ask("lyrics", now.song.id);
  tick();
}

function closeSheet() {
  sheetOpen = false;
  sheet().hidden = true;
  document.body.classList.remove("sheet-open");
}

let speeds: { speed: string; pitch: string; autoMix: string; crossfadeSec: string } | undefined;
/** The values speed and pitch offer (the core's `settings_model::specs`). */
let rates: { speed: string[]; pitch: string[] } | undefined;

function drawSheet() {
  const s = now?.song;
  const el = sheet();
  if (!s || !now) {
    el.replaceChildren(h("div", { class: "sheet-empty" }, W.nothingPlaying));
    return;
  }
  const playing = now.state === "Playing";
  const seek = h("input", { type: "range", class: "seek", min: 0, max: Math.max(1, s.duration * 1000), value: Math.round(position()), "aria-label": "Position" }) as HTMLInputElement;
  seek.oninput = () => (seekDragging = true);
  seek.onchange = () => {
    seekDragging = false;
    void control("seek", Number(seek.value));
  };
  const side =
    tab === "lyrics"
      ? h("div", { class: "lyrics", id: "lyrics" }, ...lyricLines())
      : tab === "jam"
        ? jamSide()
        : h(
          "ol",
          { class: "queue" },
          ...now.queue.map((q, i) =>
            h(
              "li",
              { class: i === now!.current ? "current" : "", on: { click: () => void control("jump", i) } },
              cover(q.cover, "cover tiny"),
              h("div", { class: "row-text" }, h("span", { class: "title" }, q.title ?? q.id), h("span", { class: "sub" }, q.artist ?? "")),
              i === now!.current ? null : iconButton("close", W.remove, () => void control("remove", i), "plain"),
            ),
          ),
        );
  el.replaceChildren(
    h(
      "div",
      { class: "sheet-inner" },
      h("button", { class: "grabber", "aria-label": "Close", on: { click: closeSheet } }, h("span")),
      h(
        "div",
        { class: "sheet-main" },
        cover(s.cover, `cover huge${playing ? " playing" : ""}`),
        h(
          "div",
          { class: "now-text" },
          h("div", { class: "title-row" }, h("span", { class: "title" }, s.title), heart("song", s.id, !!(s as Song & { starred?: boolean }).starred)),
          h("a", { class: "sub link", href: s.artistId ? `#/artist/${s.artistId}` : "#", on: { click: () => closeSheet() } }, s.artist),
          now.mixing ? h("span", { class: "badge" }, W.autoMix) : null,
          (now as Now & { jam?: boolean }).jam ? h("span", { class: "badge" }, W.jam) : null,
        ),
        seek,
        h("div", { class: "times" }, h("span", { id: "at" }, W.time(position())), h("span", { id: "left" }, `-${W.time(s.duration * 1000 - position())}`)),
        h(
          "div",
          { class: "transport" },
          iconButton("shuffle", W.shuffle, () => void control("shuffle", !now!.shuffle), now.shuffle ? "plain on" : "plain"),
          iconButton("previous", W.previous, () => void control("previous"), "plain big"),
          iconButton(playing ? "pause" : "play", playing ? W.pause : W.play, () => void control("toggle"), "plain huge"),
          iconButton("next", W.next, () => void control("next"), "plain big"),
          iconButton(now.repeat === 1 ? "repeat-one" : "repeat", W.repeat, () => void control("cycle_repeat"), now.repeat ? "plain on" : "plain"),
        ),
        soundRow(),
        h(
          "div",
          { class: "tabs" },
          h("button", { class: tab === "lyrics" ? "tab on" : "tab", on: { click: () => switchTab("lyrics") } }, icon("lyrics"), W.lyrics),
          h("button", { class: tab === "queue" ? "tab on" : "tab", on: { click: () => switchTab("queue") } }, icon("queue"), W.queue),
          h("button", { class: tab === "jam" ? "tab on" : "tab", on: { click: () => switchTab("jam") } }, icon("jam"), W.jam),
        ),
      ),
      h("div", { class: "sheet-side" }, side),
    ),
  );
}

/** The jam's panel, filled in once read. */
function jamSide(): HTMLElement {
  const box = h("div", { class: "jam-side" });
  void panel(() => sheetOpen && tab === "jam" && drawSheet()).then((p) => box.replaceChildren(p));
  return box;
}

/** AutoMix, speed and pitch, set where the session keeps them. */
function soundRow(): HTMLElement {
  const row = h("div", { class: "sound" });
  const draw = () => {
    if (!speeds || !rates) return;
    const automix = h("button", { class: speeds.autoMix === "true" ? "chip on" : "chip", on: { click: () => setSound("autoMix", speeds!.autoMix === "true" ? "false" : "true") } }, W.autoMix);
    const pick = (name: "speed" | "pitch", label: string) => {
      const values = rates![name];
      const sel = h("select", { "aria-label": label, on: { change: (e) => setSound(name, (e.target as HTMLSelectElement).value) } }, ...values.map((v) => h("option", { value: v, selected: Number(v) === Number(speeds![name]) }, `${label} ${v}×`))) as HTMLSelectElement;
      return sel;
    };
    row.replaceChildren(automix, pick("speed", W.speed), pick("pitch", W.pitch));
  };
  if (speeds && rates) draw();
  else
    void Promise.all([ask<string>("settings", "speed,pitch,autoMix,crossfadeSec"), ask<string>("options", "speed,pitch")]).then(([v, o]) => {
      speeds = JSON.parse(v);
      rates = JSON.parse(o);
      draw();
    });
  return row;
}

async function setSound(name: string, value: string) {
  await ask("setting", name, value);
  speeds = JSON.parse(await ask<string>("settings", "speed,pitch,autoMix,crossfadeSec"));
  drawSheet();
}

function switchTab(t: "lyrics" | "queue" | "jam") {
  tab = t;
  if (t === "lyrics" && now?.song && !lyrics) void ask("lyrics", now.song.id);
  drawSheet();
}

function lyricLines(): HTMLElement[] {
  if (!lyrics || lyrics.song !== now?.song?.id) return [h("p", { class: "lyrics-none" }, W.lyricsLoading)];
  const lines = lyrics.lyrics.lines ?? [];
  if (lines.length === 0) return [h("p", { class: "lyrics-none" }, W.noLyrics)];
  if (!lyrics.lyrics.synced) return lines.map((l) => h("p", { class: "line plain" }, l.text || "♪"));
  return lines.map((l, i) =>
    h(
      "p",
      { class: i === lit ? "line on" : "line", "data-i": i, tabindex: 0, role: "button", on: { click: () => void tapLine(i), keydown: (e) => (e as KeyboardEvent).key === "Enter" && void tapLine(i) } },
      l.text || "♪",
    ),
  );
}

/** Plays from line `i`, lit at once. */
async function tapLine(i: number) {
  const tap = JSON.parse(await ask<string>("lyrics_tap", i));
  if (!tap) return;
  facts.tapped = tap;
  light(tap.active);
  await refresh();
}

let seekDragging = false;
let ticking = false;
let lit = -2;
let lyricsAsked = 0;

/** The times and the seek bar at the place now. */
function drawPlace() {
  const s = now?.song;
  if (!s) return;
  const at = position();
  const t = document.getElementById("at");
  if (t) t.textContent = W.time(at);
  const left = document.getElementById("left");
  if (left) left.textContent = `-${W.time(s.duration * 1000 - at)}`;
  const seek = document.querySelector<HTMLInputElement>(".seek");
  if (seek && !seekDragging) seek.value = String(Math.round(at));
}

/** While the sheet shows, music plays and the page is in sight: the place moves and the sung line is lit,
 * each frame. Nothing runs otherwise. */
function tick() {
  const moving = () => sheetOpen && now?.state === "Playing" && !document.hidden;
  if (ticking || !moving()) return;
  ticking = true;
  const frame = async () => {
    if (!moving()) {
      ticking = false;
      return;
    }
    drawPlace();
    if (tab === "lyrics" && lyrics?.lyrics.synced && performance.now() >= lyricsAsked) {
      // A worker that went (a crash) answers nothing: the place moves on regardless.
      const step = JSON.parse(await ask<string>("lyrics_now").catch(() => "null"));
      if (step) {
        lyricsAsked = performance.now() + Math.max(30, Math.min(step.wait || 250, 500));
        light(step.active);
      }
    }
    requestAnimationFrame(() => void frame());
  };
  requestAnimationFrame(() => void frame());
}

function light(active: number) {
  if (active === lit) return;
  lit = active;
  facts.lyricLine = active;
  const box = document.getElementById("lyrics");
  if (!box) return;
  box.querySelectorAll(".line.on").forEach((p) => p.classList.remove("on"));
  const p = box.querySelector<HTMLElement>(`[data-i="${active}"]`);
  if (p) {
    p.classList.add("on");
    p.scrollIntoView({ block: "center", behavior: "smooth" });
  }
}

export function setup() {
  document.addEventListener("visibilitychange", tick);
  // A heart or a playlist changed: the song playing is read again with it.
  window.addEventListener("nori-changed", () => void refresh());
  // The player worker crashed and started again over the saved queue: it plays on if it played.
  onRestart(() => {
    const [was, song, at] = [now?.state === "Playing", now?.id, position()];
    say(W.restarted);
    facts.restartedAt = { song, ms: Math.round(at) };
    void (async () => {
      await refresh();
      // The saved queue knows the song; the place in it is the one this page saw last.
      if (song && now?.id === song) await control("seek", at);
      if (was && now?.state !== "Playing") await control("toggle");
    })();
  });
  onSaid((s) => {
    if (s.kind === "engine") void refresh();
    else if (s.kind === "remote") {
      void refresh();
      const side = document.querySelector(".jam-side");
      // Drawn again only when the jam changed, not on every word from the relay.
      if (sheetOpen && tab === "jam" && side)
        void ask<string>("jam").then((j) => {
          if (j === jamShown) return;
          jamShown = j;
          void panel(() => drawSheet()).then((p) => side.replaceChildren(p));
        });
    }
    else if (s.kind === "lyrics") {
      lyrics = { song: s.song as string, lyrics: s.lyrics as Lyrics };
      lit = -2;
      facts.lyrics = { song: s.song, lines: (s.lyrics as Lyrics).lines?.length ?? 0, synced: (s.lyrics as Lyrics).synced };
      if (sheetOpen && tab === "lyrics") drawSheet();
    } else if (s.kind === "note") {
      const n = s.note as { what: string; songs?: number; next?: boolean };
      if (n.what === "queued") say(W.queued(n.songs ?? 0, !!n.next));
      else if (n.what === "songsFailed") say(W.songsFailed);
    } else if (s.kind === "reachable" && !s.ok) say(W.offline);
  });
  onHeard((heard) => {
    facts.heard = { seconds: heard.played / (audio?.sampleRate ?? 48000), dry: heard.dry, dryAt: heard.dryAt, peak: heard.peak, silences: heard.silences, lastLoud: heard.lastLoud, soundAt: heard.soundAt, freshAt: heard.freshAt };
  });
  if ("mediaSession" in navigator) {
    navigator.mediaSession.setActionHandler("play", () => void control("toggle"));
    navigator.mediaSession.setActionHandler("pause", () => void control("toggle"));
    navigator.mediaSession.setActionHandler("nexttrack", () => void control("next"));
    navigator.mediaSession.setActionHandler("previoustrack", () => void control("previous"));
  }
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && sheetOpen) closeSheet();
    if (e.key === " " && player && !(e.target instanceof HTMLInputElement)) {
      e.preventDefault();
      void control("toggle");
    }
  });
}

/** What plays now, for a check. */
export function nowPlaying(): Now | undefined {
  return now;
}
