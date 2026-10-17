// The sound card: plays the interleaved chunks the worker sends over this node's port, and says how
// many frames it played so the worker keeps it fed. Nothing here decides anything.

declare abstract class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor();
}
declare function registerProcessor(name: string, processor: unknown): void;

/** Frames between reports to the worker. */
const REPORT = 2048;

class Out extends AudioWorkletProcessor {
  private chunks: { pcm: Float32Array; channels: number }[] = [];
  private at = 0;
  private played = 0;
  private reported = 0;
  private dry = 0;
  private peak = 0;
  private worker?: MessagePort;

  constructor() {
    super();
    // The page hands over the port the worker writes to.
    this.port.onmessage = (e: MessageEvent) => {
      this.worker = e.data as MessagePort;
      this.worker.onmessage = (m: MessageEvent) => this.chunks.push(m.data);
    };
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const out = outputs[0];
    const frames = out[0].length;
    let made = 0;
    while (made < frames && this.chunks.length > 0) {
      const { pcm, channels } = this.chunks[0];
      const left = pcm.length / channels - this.at;
      const n = Math.min(left, frames - made);
      for (let f = 0; f < n; f++) {
        for (let c = 0; c < out.length; c++) {
          const v = pcm[(this.at + f) * channels + Math.min(c, channels - 1)];
          out[c][made + f] = v;
          if (v > this.peak) this.peak = v;
        }
      }
      made += n;
      this.at += n;
      if (this.at * channels >= pcm.length) {
        this.chunks.shift();
        this.at = 0;
      }
    }
    if (made < frames && this.played > 0) this.dry++;
    this.played += made;
    if (this.worker && (this.played - this.reported >= REPORT || (made < frames && this.played > this.reported))) {
      this.reported = this.played;
      this.worker.postMessage({ played: this.played, dry: this.dry, peak: this.peak });
    }
    return true;
  }
}

registerProcessor("nori-out", Out);
