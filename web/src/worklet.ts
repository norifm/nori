// The sound card: plays the interleaved chunks the worker sends over this node's port, and says how
// many frames it played so the worker keeps it fed. Nothing here decides anything.

declare abstract class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor();
}
declare function registerProcessor(name: string, processor: unknown): void;
declare const sampleRate: number;

/** Frames between reports to the worker. */
const REPORT = 2048;
/** Frames held before playing starts, and again after running dry: a little slack for the worker. */
const START = 4096;

type Chunk = { pcm: Float32Array; channels: number };
/** From the worker: music, or whether the player is holding (paused or stopped: running out is no fault). */
type Message = Chunk | { held: boolean };

class Out extends AudioWorkletProcessor {
  private chunks: Chunk[] = [];
  private queued = 0;
  private at = 0;
  private played = 0;
  private reported = 0;
  /** Playing, as opposed to filling up to START. */
  private running = false;
  private held = true;
  /** Times the music ran out while playing, and the seconds played at the first of them. */
  private dry = 0;
  private dryAt: number[] = [];
  private peak = 0;
  /** Silent frames in a row now, and the last silences over 20 ms as [seconds played at their start, ms
   * long]: a gap between songs shows. */
  private quiet = 0;
  private silences: [number, number][] = [];
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
      this.worker.onmessage = (m: MessageEvent) => this.take(m.data as Message);
    };
  }

  private take(m: Message) {
    if ("held" in m) {
      this.held = m.held;
      return;
    }
    this.chunks.push(m);
    this.queued += m.pcm.length / m.channels;
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const out = outputs[0];
    const frames = out[0].length;
    if (!this.running && (this.queued >= START || (this.held && this.queued > 0))) this.running = true;
    let made = 0;
    while (this.running && made < frames && this.chunks.length > 0) {
      const { pcm, channels } = this.chunks[0];
      const left = pcm.length / channels - this.at;
      const n = Math.min(left, frames - made);
      for (let f = 0; f < n; f++) {
        let loud = false;
        for (let c = 0; c < out.length; c++) {
          const v = pcm[(this.at + f) * channels + Math.min(c, channels - 1)];
          out[c][made + f] = v;
          if (v > this.peak) this.peak = v;
          if (v > 1e-4 || v < -1e-4) loud = true;
        }
        if (!loud) this.quiet++;
        else if (this.quiet > 0) {
          if (this.quiet > sampleRate / 50) {
            const at = this.played + made + f - this.quiet;
            this.silences.push([Math.round((at / sampleRate) * 1000) / 1000, Math.round((this.quiet / sampleRate) * 1000)]);
            if (this.silences.length > 32) this.silences.shift();
          }
          this.quiet = 0;
        }
      }
      made += n;
      this.at += n;
      if (this.at * channels >= pcm.length) {
        this.chunks.shift();
        this.at = 0;
      }
    }
    this.queued -= made;
    if (this.running && made < frames) {
      this.running = false;
      if (!this.held) {
        this.dry++;
        if (this.dryAt.length < 32) this.dryAt.push(Math.round((this.played / sampleRate) * 1000) / 1000);
      }
    }
    this.played += made;
    if (this.worker && (this.played - this.reported >= REPORT || (made < frames && this.played > this.reported))) {
      this.reported = this.played;
      this.worker.postMessage({ played: this.played, dry: this.dry, dryAt: this.dryAt, peak: this.peak, silences: this.silences });
    }
    return true;
  }
}

registerProcessor("nori-out", Out);
