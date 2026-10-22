// The sound card: plays the interleaved chunks the worker sends over this node's port, does what the
// engine's output tells it (pause, resume, flush, fade), and says how many frames it played and dropped
// so the engine knows what was heard. Nothing here decides anything.

declare abstract class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor();
}
declare function registerProcessor(name: string, processor: unknown): void;
declare const sampleRate: number;
declare const currentTime: number;

/** Frames between reports to the worker. */
const REPORT = 2048;
/** Frames held before playing starts, and again after running dry: a little slack for the worker. */
const START = 4096;
/** The fade a pause, a resume or a flush makes so nothing clicks, per frame (15 ms). */
const EDGE_STEP = 1 / (0.015 * sampleRate);

type Chunk = { pcm: Float32Array; channels: number };
/** From the engine's output: stop where it is keeping the music held, go on from there, drop the music
 * held (answered with the frames dropped), or fade to `to` over `ms` (from `from`, or where it is). */
type Order = { do: "pause" } | { do: "resume" } | { do: "flush" } | { do: "ramp"; from: number | null; to: number; ms: number };
/** From the worker: music, an order, whether the player is holding (paused or stopped: running out is
 * no fault), or a request for the diagnostics. */
type Message = Chunk | Order | { held: boolean } | { diagnose: true };

/** What the page's check reads: the dry spells, the peak, the silences and the sound card's clock at
 * the moments that matter. */
type Diagnostics = {
  /** Times the music ran out while playing, and the seconds played at the first of them. */
  dry: number;
  dryAt: number[];
  peak: number;
  /** Silent frames in a row now, and the last silences over 20 ms as [seconds played at their start, ms
   * long]: a gap between songs shows. */
  quiet: number;
  silences: [number, number][];
  /** On the sound card's clock: the last frame with sound, the first after more than 10 ms without, and
   * the first frame played after the last flush. */
  lastLoud: number;
  soundAt: number;
  freshAt: number;
  fresh: boolean;
};

const noDiagnostics = (): Diagnostics => ({ dry: 0, dryAt: [], peak: 0, quiet: 0, silences: [], lastLoud: 0, soundAt: 0, freshAt: 0, fresh: false });

class Out extends AudioWorkletProcessor {
  private chunks: Chunk[] = [];
  private queued = 0;
  private at = 0;
  private played = 0;
  private reported = 0;
  /** Playing, as opposed to filling up to START. */
  private running = false;
  private held = true;
  /** Paused: stopped where it is, once the edge has closed. */
  private paused = true;
  /** Flushes told: the music held goes once the edge has closed. */
  private flushes = 0;
  /** The short fade of a pause or flush: 0 closed, 1 open. */
  private edge = 0;
  /** The engine's fade: the level, where it goes and its step per frame. */
  private level = 1;
  private levelTo = 1;
  private levelStep = 0;
  private diag = noDiagnostics();
  private worker?: MessagePort;

  constructor() {
    super();
    // The page hands over the port the worker writes to.
    this.port.onmessage = (e: MessageEvent) => {
      // A new worker (the first, or one started again after a crash) counts from nothing.
      this.worker = e.data as MessagePort;
      this.chunks = [];
      this.queued = 0;
      this.at = 0;
      this.played = 0;
      this.reported = 0;
      this.running = false;
      this.held = true;
      this.diag = noDiagnostics();
      this.paused = true;
      this.flushes = 0;
      this.edge = 0;
      this.level = this.levelTo = 1;
      this.levelStep = 0;
      this.worker.onmessage = (m: MessageEvent) => this.take(m.data as Message);
    };
  }

  private take(m: Message) {
    if ("held" in m) this.held = m.held;
    else if ("diagnose" in m) this.worker?.postMessage({ heard: { played: this.played, ...this.diag } });
    else if ("pcm" in m) {
      this.chunks.push(m);
      this.queued += m.pcm.length / m.channels;
    } else if (m.do === "pause") this.paused = true;
    else if (m.do === "resume") this.paused = false;
    else if (m.do === "flush") this.flushes++;
    else {
      if (m.from !== null) this.level = m.from;
      this.levelTo = m.to;
      this.levelStep = (m.to - this.level) / Math.max(1, (m.ms * sampleRate) / 1000);
    }
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const out = outputs[0];
    const d = this.diag;
    const frames = out[0].length;
    if (!this.running && (this.queued >= START || (this.held && this.queued > 0))) this.running = true;
    const closing = this.paused || this.flushes > 0;
    let made = 0;
    while (this.running && made < frames && this.chunks.length > 0 && !(closing && this.edge === 0)) {
      const { pcm, channels } = this.chunks[0];
      const n = Math.min(pcm.length / channels - this.at, frames - made);
      let f = 0;
      for (; f < n; f++) {
        this.edge = closing ? Math.max(0, this.edge - EDGE_STEP) : Math.min(1, this.edge + EDGE_STEP);
        if (closing && this.edge === 0) break;
        if (this.level !== this.levelTo) {
          this.level += this.levelStep;
          if ((this.levelStep >= 0 && this.level >= this.levelTo) || (this.levelStep < 0 && this.level <= this.levelTo)) this.level = this.levelTo;
        }
        const gain = this.level * this.edge;
        const t = currentTime + (made + f) / sampleRate;
        if (d.fresh) {
          d.fresh = false;
          d.freshAt = t;
        }
        let loud = false;
        for (let c = 0; c < out.length; c++) {
          const v = pcm[(this.at + f) * channels + Math.min(c, channels - 1)] * gain;
          out[c][made + f] = v;
          if (v > d.peak) d.peak = v;
          if (v > 1e-4 || v < -1e-4) loud = true;
        }
        if (loud) {
          if (t - d.lastLoud > 0.01) d.soundAt = t;
          d.lastLoud = t;
        }
        if (!loud) d.quiet++;
        else if (d.quiet > 0) {
          if (d.quiet > sampleRate / 50) {
            const at = this.played + made + f - d.quiet;
            d.silences.push([Math.round((at / sampleRate) * 1000) / 1000, Math.round((d.quiet / sampleRate) * 1000)]);
            if (d.silences.length > 32) d.silences.shift();
          }
          d.quiet = 0;
        }
      }
      made += f;
      this.at += f;
      if (this.at * channels >= pcm.length) {
        this.chunks.shift();
        this.at = 0;
      }
    }
    this.queued -= made;
    this.played += made;
    if (this.running && made < frames && !closing) {
      this.running = false;
      if (!this.held) {
        d.dry++;
        if (d.dryAt.length < 32) d.dryAt.push(Math.round((this.played / sampleRate) * 1000) / 1000);
      }
    }
    // Closed, or nothing sounding to close: the music held goes, and what follows fades in.
    if (this.flushes > 0 && (this.edge === 0 || !this.running || this.chunks.length === 0)) {
      const [dropped, flushes] = [this.queued, this.flushes];
      this.chunks = [];
      this.queued = 0;
      this.at = 0;
      this.running = false;
      this.edge = 0;
      d.fresh = true;
      this.flushes = 0;
      this.report({ dropped, flushes });
    } else if (this.played - this.reported >= REPORT || (made < frames && this.played > this.reported)) this.report();
    return true;
  }

  /** Tells the worker what was played, and with `flushed` the flushes done and the frames they dropped.
   * The diagnostics wait until it asks for them. */
  private report(flushed?: { dropped: number; flushes: number }) {
    if (!this.worker) return;
    this.reported = this.played;
    this.worker.postMessage({ played: this.played, flushed });
  }
}

registerProcessor("nori-out", Out);
