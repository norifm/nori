// The player as the page shows it: what plays (this browser's playback, another device's while it plays,
// or a jam's), where it is, the lyrics, and the sound card. What plays and how is the session's (in the
// worker); this reads it and asks.

import * as rpc from "../rpc";
import { ask, facts } from "../rpc";
import workletUrl from "../worklet.ts?worker&url";
import { ui } from "./ui.svelte";
import { W } from "../words";
import type { Now, Shown, Song } from "./types";

/** Where the song was at a moment of the page's clock, and how fast it runs on from there (0: it holds). */
export type Anchor = { ms: number; at: number; pace: number; length: number };

/** The settings the player's own controls change. */

class Player {
  now = $state.raw<Now | null>(null);
  anchor = $state.raw<Anchor>({ ms: 0, at: 0, pace: 0, length: 0 });
  /** The lyrics shown, and the lit line and how much of it is sung. */
  shown = $state.raw<Shown | null>(null);
  line = $state(-1);
  /** How much of the lit line is sung; -1 when its words do not light up one by one. */
  sung = $state(-1);
  /** The player worker answers (the core worker answers only the library before). */
  up = $state(false);
  /** The session plays here and the browser holds the sound card until the page is clicked. */
  held = $state(false);

  song = $derived(this.now?.song ?? null);
  playing = $derived(this.now?.playing ?? false);
  /** Another device plays: the controls reach it, nothing plays here. */
  elsewhere = $derived(!!this.now?.device);

  private audio?: AudioContext;
  private made?: Promise<void>;
  /** `now` as last drawn, without its position: drawn again only when the rest changes. */
  private key = "";

  /** Where the song is at `t` on the page's clock. */
  position(t = performance.now()): number {
    const a = this.anchor;
    return Math.max(0, Math.min(a.ms + (t - a.at) * a.pace, a.length || Infinity));
  }

  /** The sound card, made on the first Play (a browser starts audio only after a click on the page):
   * an AudioWorklet the player worker feeds over its own port. */
  soundCard(): Promise<void> {
    if (this.audio) {
      void this.audio.resume();
      return this.made!;
    }
    const ctx = (this.audio = new AudioContext({ latencyHint: "playback" }));
    void ctx.resume();
    ctx.onstatechange = () => (this.held = ctx.state === "suspended" && this.now?.state === "Playing");
    this.made = (async () => {
      await ctx.audioWorklet.addModule(workletUrl);
      const node = new AudioWorkletNode(ctx, "nori-out", { outputChannelCount: [2] });
      node.connect(ctx.destination);
      facts.rate = ctx.sampleRate;
      // The sound card's clock, which the worklet's times are on.
      Object.defineProperty(facts, "audioTime", { get: () => ctx.currentTime, enumerable: true, configurable: true });
      if (!rpc.player) ui.say(W.loadingPlayer);
      // A new line each time the player worker starts (again, after a crash).
      await rpc.soundTo(() => {
        const line = new MessageChannel();
        node.port.postMessage(line.port1, [line.port1]);
        return { port: line.port2, rate: ctx.sampleRate, ear: ctx.baseLatency + (ctx.outputLatency || 0) };
      });
      if (ui.toast === W.loadingPlayer) ui.say("");
    })().catch((e) => {
      // A failed setup is made again by the next Play.
      this.audio = this.made = undefined;
      void ctx.close();
      throw e;
    });
    return this.made;
  }

  /** A click anywhere lets a held sound card play. */
  unhold() {
    if (this.audio?.state === "suspended") void this.audio.resume();
  }

  /** Plays `songs` from `start`; `kind` and `id` name the page they are the list of. Call from a click. */
  async play(songs: Song[], start: number, shuffle = false, kind = "", id = "") {
    await this.soundCard();
    await ask("play", JSON.stringify(songs), start, shuffle, kind, id);
    await this.refresh();
  }

  async enqueue(songs: Song[], next: boolean) {
    await this.soundCard();
    await ask("enqueue", JSON.stringify(songs), next);
    await this.refresh();
  }

  /** A radio or an instant mix from `song`. */
  async mix(song: Song, radio: boolean) {
    await this.soundCard();
    if ((await ask<number>("mix", song.id, radio)) === 0) ui.say(W.mixEmpty);
    await this.refresh();
  }

  /** One of the session's controls (`toggle`, `next`, `seek`, ...). */
  async control(call: string, ...args: unknown[]) {
    if (!rpc.player) return;
    // Playing here needs the sound card; a press is the click that lets it start.
    if (call === "toggle" && !this.elsewhere) await this.soundCard();
    await ask(call, ...args);
    await this.refresh();
  }

  async seek(ms: number) {
    // Shown there at once; the session's answer follows.
    this.anchor = { ...this.anchor, ms, at: performance.now() };
    await this.control("seek", ms);
  }

  /** Reads what plays; draws it again only when more than the position moved. */
  async refresh() {
    if (!rpc.player) return;
    const n = JSON.parse(await ask<string>("now")) as Now | null;
    const t = performance.now();
    const running = n?.state === "Playing" && !n.buffering;
    const a: Anchor = { ms: n?.positionMs ?? 0, at: t, pace: running ? n!.pace || 1 : 0, length: (n?.song?.duration ?? 0) * 1000 };
    // The position as it runs on is kept unless it is off by more than a frame or two.
    const was = this.anchor;
    if (a.pace !== was.pace || a.length !== was.length || Math.abs(this.position(t) - a.ms) > 40) this.anchor = a;
    const key = JSON.stringify(n && { ...n, positionMs: 0 });
    if (key === this.key) return;
    this.key = key;
    const before = this.now?.id;
    this.now = n;
    this.held = this.audio?.state === "suspended" && n?.state === "Playing";
    if (n?.id !== before) this.songChanged();
    // Music handed over from another device plays here: the sound card is made for it.
    if (n?.state === "Playing" && !n.device && !n.jam && !this.audio) void this.soundCard().catch(() => {});
  }

  private songChanged() {
    const s = this.song;
    if (s && "mediaSession" in navigator) {
      navigator.mediaSession.metadata = new MediaMetadata({ title: s.title, artist: s.artist, album: s.album, artwork: s.cover ? [{ src: s.cover, sizes: "300x300" }] : [] });
    }
  }

  /** Asks for the lyrics of the song playing, unless they are shown. */
  wantLyrics() {
    const id = this.song?.id;
    if (id && this.shown?.song !== id && rpc.player) void ask("lyrics", id);
  }

  /** The lyrics' state now, lit; ms until it changes (0: not until the position jumps). */
  async lyricsStep(frameMs: number): Promise<number> {
    const s = JSON.parse(await ask<string>("lyrics_now", frameMs)) as { song: string; active: number; swept: boolean; sung: number; wait: number } | null;
    if (!s || s.song !== this.shown?.song) return 0;
    this.line = s.active;
    this.sung = s.swept ? s.sung : -1;
    facts.lyricLine = s.active;
    return s.wait;
  }

  /** Plays from lyric line `line`, lit at once (nothing for a jam guest who asks the host). */
  async tapLine(line: number) {
    const tap = JSON.parse(await ask<string>("lyrics_tap", line)) as { active: number; swept: boolean; ms: number } | null;
    if (!tap) return;
    facts.tapped = tap;
    this.line = tap.active;
    this.sung = tap.swept ? 0 : -1;
    facts.lyricLine = tap.active;
    await this.refresh();
  }

  /** Listens to the worker: the session's reports, what the sound card heard, a restart. */
  listen() {
    rpc.onPlayerUp(() => {
      this.up = true;
      void this.refresh();
    });
    rpc.onRestart(() => {
      // The player worker crashed and started again over the saved queue: it plays on if it played.
      const [was, song, at] = [this.now?.state === "Playing", this.now?.id, this.position()];
      ui.say(W.restarted);
      facts.restartedAt = { song, ms: Math.round(at) };
      void (async () => {
        await this.refresh();
        if (song && this.now?.id === song) await this.control("seek", at);
        if (was && this.now?.state !== "Playing") await this.control("toggle");
      })();
    });
    rpc.onSaid((s) => {
      if (s.kind === "engine" || s.kind === "remote" || s.kind === "starred") void this.refresh();
      else if (s.kind === "lyrics") {
        const shown = s as unknown as Shown;
        if (shown.song !== this.shown?.song) this.line = -1;
        this.shown = { song: shown.song, lyrics: shown.lyrics, origin: shown.origin, timing: shown.timing };
        facts.lyrics = { song: shown.song, lines: shown.lyrics.lines?.length ?? 0, synced: shown.lyrics.synced, timing: shown.timing };
      } else if (s.kind === "note") {
        const n = s.note as { what: string; songs?: number; next?: boolean };
        if (n.what === "queued") ui.say(W.queued(n.songs ?? 0, !!n.next));
        else if (n.what === "songsFailed") ui.say(W.songsFailed);
      } else if (s.kind === "reachable" && !s.ok) ui.say(W.offline);
    });
    // What the sound card heard, asked for when read (the headless check's), so nothing is sent while playing.
    facts.heard = async () => {
      const heard = rpc.player ? await ask<rpc.Heard | null>("heard") : null;
      return heard && { seconds: heard.played / (this.audio?.sampleRate ?? 48000), dry: heard.dry, dryAt: heard.dryAt, peak: heard.peak, silences: heard.silences, lastLoud: heard.lastLoud, soundAt: heard.soundAt, freshAt: heard.freshAt };
    };
    if ("mediaSession" in navigator) {
      const m = navigator.mediaSession;
      m.setActionHandler("play", () => void this.control("toggle"));
      m.setActionHandler("pause", () => void this.control("toggle"));
      m.setActionHandler("nexttrack", () => void this.control("next"));
      m.setActionHandler("previoustrack", () => void this.control("previous"));
      m.setActionHandler("seekto", (d) => d.seekTime !== undefined && void this.seek(d.seekTime * 1000));
    }
    addEventListener("pointerdown", () => this.unhold(), { capture: true });
    addEventListener("keydown", (e) => {
      const t = e.target as HTMLElement;
      if (e.key === " " && rpc.player && !(t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement || t.isContentEditable)) {
        e.preventDefault();
        void this.control("toggle");
      }
      if (e.key === "Escape") {
        if (ui.lyricsInfo) ui.lyricsInfo = false;
        else if (ui.full) ui.full = false;
      }
    });
  }
}

export const player = new Player();

/** The lock screen's position, kept with what the page shows. */
$effect.root(() => {
  $effect(() => {
    const a = player.anchor;
    const n = player.now;
    if (!("mediaSession" in navigator)) return;
    navigator.mediaSession.playbackState = n ? (n.playing ? "playing" : "paused") : "none";
    if (a.length > 0) {
      try {
        navigator.mediaSession.setPositionState({ duration: a.length / 1000, position: Math.min(player.position(), a.length) / 1000, playbackRate: a.pace || 1 });
      } catch {
        // A position past the length while a song changes.
      }
    }
  });
});
