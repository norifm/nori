// The installed plugins: what's on, each one's worker and sound stage, and what they answer. Loaded only
// once a plugin is installed, or by the Plugins page.

import { ask, onPlayerUp, onRestart } from "../rpc";
import { player } from "../state/player.svelte";
import { sleeves } from "../state/sleeve.svelte";
import { ui } from "../state/ui.svelte";
import * as db from "./db";
import { overridesIn, type Call, type Start } from "./runner";
import { stageModule } from "./stage";
import type { Code, Installed, Row, Value, WebPlugin } from "./types";
import { numbers, witValue } from "./types";

/** What says a plugin is installed, read at the page's start so none costs nothing. */
const MARK = "nori-plugins";
const PROXY = new URL("plugin-http", document.baseURI).href;

export const anyInstalled = () => localStorage.getItem(MARK) === "1";

/** Whether this page's server carries a plugin's requests. */
async function proxyAnswers() {
  const r = await fetch(`${PROXY}?ping`).catch(() => null);
  return r?.headers.get("X-Plugin-Http") === "1";
}

/** A running plugin's thread. */
class Running {
  private worker = new Worker(new URL("./runner.ts", import.meta.url), { type: "module" });
  private next = 1;
  private calls = new Map<number, { resolve(v: unknown): void; reject(e: Error): void }>();
  started: Promise<void>;
  status = $state<Record<string, string>>({});
  shown = $state<Record<string, { shown: boolean; enabled: boolean }>>({});
  overrides: [string, string][];

  constructor(
    i: Installed,
    code: Code,
    data: Map<string, Uint8Array>,
    proxy: string | null,
    overridden: () => void,
  ) {
    const id = i.plugin.id;
    this.overrides = overridesIn(data);
    this.started = new Promise((started, failed) => {
      this.worker.onmessage = (e: MessageEvent) => {
        const m = e.data;
        if (m.started) started();
        else if (m.failed) failed(new Error(m.failed));
        else if (m.n) {
          const c = this.calls.get(m.n)!;
          this.calls.delete(m.n);
          if (m.error) c.reject(new Error(m.error));
          else c.resolve(m.value);
        } else if (m.set !== undefined) void db.put(id, m.set, m.value);
        else if (m.unset !== undefined) void db.drop(id, m.unset);
        else if (m.log) console.info(`[${id}]`, m.log);
        else if (m.notice) ui.say(m.notice.words);
        else if (m.status) this.status[m.status.row] = m.status.words;
        else if (m.shown) this.shown[m.shown.row] = m.shown;
        else if (m.overrides) {
          this.overrides = m.overrides;
          overridden();
        }
      };
    });
    const { plugin, values } = $state.snapshot(i);
    const start: Start = { code, net: plugin.net, overrides: plugin.overrides, proxy, rows: plugin.rows, values, data };
    this.worker.postMessage(start);
  }

  call<T>(at: Call["at"], call: string, ...args: unknown[]): Promise<T> {
    const n = this.next++;
    return new Promise((resolve, reject) => {
      this.calls.set(n, { resolve: resolve as (v: unknown) => void, reject });
      this.worker.postMessage({ n, at, call, args } satisfies Call);
    });
  }

  stop() {
    this.worker.terminate();
  }
}

/** The plugin API's `song`, from the page's. */
function song(s: NonNullable<typeof player.song>) {
  return {
    id: s.id,
    title: s.title,
    artist: s.artist,
    artistId: s.artistId,
    artists: [],
    album: s.album,
    albumId: s.albumId,
    cover: s.cover,
    duration: Math.round(s.duration),
    track: s.track ?? 0,
    disc: 0,
    year: 0,
    genre: undefined,
    starred: !!s.starred,
    external: s.id.startsWith("ext-"),
    explicit: "unknown",
    replayGain: undefined,
    bpm: 0,
    rating: 0,
  };
}

type Sleeve = { tag: "still"; val: string } | { tag: "loop"; val: { video: string; poster?: string } };

class Plugins {
  installed = $state<Installed[]>([]);
  proxied = $state(false);
  private running = new Map<string, Running>();
  /** Stages added to the worklet, as `id` at `version`: a worklet can't take a module away. */
  private added = new Set<string>();
  private loaded?: Promise<void>;

  /** The list as kept, and everything that's on started. */
  load(): Promise<void> {
    return (this.loaded ??= (async () => {
      this.proxied = await proxyAnswers();
      this.installed = await db.list();
      sleeves.failed = (id, status) => this.sleeveFailed(id, status);
      player.onCard = () => void this.stages();
      // The core keeps them in memory, and each worker has its own.
      onPlayerUp(() => this.lay());
      onRestart(() => this.lay());
      $effect.root(() => {
        $effect(() => void this.asked(player.song));
        $effect(() => {
          const event = player.playing ? "playback-started" : "playback-paused";
          for (const r of this.withHooks()) void r.call("hooks", "on", { tag: event });
        });
      });
      for (const i of this.installed) if (i.on) await this.begin(i);
      await this.stages();
      this.lay();
      await this.asked(player.song);
    })());
  }

  find(id: string) {
    return this.installed.find((i) => i.plugin.id === id);
  }

  runningOf(id: string) {
    return this.running.get(id);
  }

  private withHooks() {
    return this.installed.filter((i) => i.on && i.plugin.points.includes("hooks")).flatMap((i) => this.running.get(i.plugin.id) ?? []);
  }

  private needsWorker = (p: WebPlugin) => p.points.some((x) => x !== "sound-stage" && x !== "lifecycle") || (p.points.includes("lifecycle") && !p.stage);

  private async begin(i: Installed) {
    const id = i.plugin.id;
    if (!this.needsWorker(i.plugin) || this.running.has(id)) return;
    const [code, data] = await Promise.all([db.code(id), db.data(id)]);
    const r = new Running(i, code, data, this.proxied ? PROXY : null, () => this.lay());
    this.running.set(id, r);
    await r.started;
    if (i.plugin.points.includes("lifecycle")) await r.call("lifecycle", "start");
  }

  /** Tells the core what the plugins running override, in their order: the last says. */
  private lay() {
    const overrides = this.installed.flatMap((i) => this.running.get(i.plugin.id)?.overrides ?? []);
    void ask("plugin_overrides", JSON.stringify(overrides));
  }

  /** What's on, in the sound card, once there is one. */
  private async stages() {
    const card = player.card();
    if (!card) return;
    card.node.port.onmessage = (e) => console.warn(`plugin stage ${e.data.stageFailed}: ${e.data.error}`);
    const list = [];
    for (const i of this.installed) {
      if (!i.on || !i.plugin.stage) continue;
      const code = await db.code(i.plugin.id);
      const key = `${i.plugin.id}@${i.plugin.version}`;
      if (!this.added.has(key)) {
        const url = stageModule(key, code.js);
        await card.ctx.audioWorklet.addModule(url);
        URL.revokeObjectURL(url);
        this.added.add(key);
      }
      list.push({ id: key, cores: code.cores, numbers: numbers(i), lifecycle: i.plugin.points.includes("lifecycle") });
    }
    card.node.port.postMessage({ stages: list });
  }

  /** Asks the plugins with a sleeve for the song, and tells those listening that it changed. */
  private async asked(s: typeof player.song) {
    if (sleeves.current?.song !== s?.id) sleeves.current = null;
    for (const r of this.withHooks()) void r.call("hooks", "on", { tag: "song-changed", val: s ? song(s) : undefined });
    if (s) await this.sleeve(s);
  }

  /** The first sleeve an installed plugin gives for `s`, shown while it plays. */
  async sleeve(s: NonNullable<typeof player.song>) {
    for (const i of this.installed) {
      if (!i.on || !i.plugin.points.includes("slots")) continue;
      const answer = await this.running.get(i.plugin.id)?.call<Sleeve | undefined>("slots", "artwork", song(s)).catch((e) => console.warn(`plugin artwork: ${e}`));
      if (!answer || player.song?.id !== s.id) continue;
      sleeves.current = answer.tag === "loop" ? { song: s.id, video: answer.val.video, poster: answer.val.poster } : { song: s.id, still: answer.val };
      return sleeves.current;
    }
    return null;
  }

  private sleeveFailed(id: string, status: number) {
    const s = player.song;
    if (s?.id !== id) return;
    sleeves.current = null;
    for (const i of this.installed) if (i.on && i.plugin.points.includes("slots")) void this.running.get(i.plugin.id)?.call("slots", "artworkFailed", song(s), status).catch(() => {});
  }

  /** Keeps a package and shows it, off. What can't run here isn't kept: its reasons are returned. */
  async install(pack: Uint8Array, sha256?: string): Promise<string[]> {
    const { open } = await import("./install");
    const { plugin, code, icon } = await open(pack, this.proxied, sha256);
    if (!code) return plugin.blocks;
    this.stop(plugin.id);
    const i: Installed = { plugin, on: false, values: this.find(plugin.id)?.values ?? {}, icon };
    await db.saveCode(plugin.id, code);
    await db.save($state.snapshot(i));
    this.installed = [...this.installed.filter((x) => x.plugin.id !== plugin.id), i];
    localStorage.setItem(MARK, "1");
    return [];
  }

  /** The package at `url`, through this server when it carries requests. */
  async download(url: string): Promise<Uint8Array> {
    const r = this.proxied ? await fetch(`${PROXY}?to=${encodeURIComponent(url)}`, { headers: { "X-Plugin-Headers": "[]" } }) : await fetch(url);
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    return new Uint8Array(await r.arrayBuffer());
  }

  async setOn(id: string, on: boolean) {
    const i = this.find(id)!;
    i.on = on;
    await db.save($state.snapshot(i));
    if (on) await this.begin(i);
    else this.stop(id);
    await this.stages();
    await this.asked(player.song);
  }

  async setRow(id: string, row: Row, value: Value) {
    const i = this.find(id)!;
    i.values[row.id] = value;
    await db.save($state.snapshot(i));
    if (!i.on) return;
    if (i.plugin.stage) player.card()?.node.port.postMessage({ numbers: { id: `${id}@${i.plugin.version}`, numbers: numbers(i) } });
    if (i.plugin.points.includes("lifecycle")) await this.running.get(id)?.call("lifecycle", "changed", row.id, witValue(row, value));
  }

  async act(id: string, row: string) {
    await this.running.get(id)?.call("lifecycle", "action", row);
  }

  private stop(id: string) {
    this.running.get(id)?.stop();
    if (this.running.delete(id)) this.lay();
  }

  async remove(id: string) {
    this.stop(id);
    this.installed = this.installed.filter((i) => i.plugin.id !== id);
    await db.remove(id);
    if (this.installed.length === 0) localStorage.removeItem(MARK);
    await this.stages();
    await this.asked(player.song);
  }
}

export const plugins = new Plugins();
// For a check driving the page.
Object.assign(window, { plugins, player });
