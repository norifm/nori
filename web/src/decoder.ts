// The browser's own decoders (WebCodecs AudioDecoder) for the player module: fed a song's packets in
// order, each one's samples are kept until the module takes them back, interleaved. The worker probes
// which codecs decode here before the module starts (`probe`); the module asks for a decoder by codec.

let supported = new Set<string>();
let woken: () => void = () => {};

/** The codec strings WebCodecs is asked for, by the module's codec names. */
function codecString(codec: string, config?: Uint8Array): string {
  if (codec === "aac") {
    // The object type from the AudioSpecificConfig: 2 LC, 5 SBR, 29 PS.
    const aot = config && config.length > 0 ? config[0] >> 3 : 2;
    return `mp4a.40.${aot}`;
  }
  return codec;
}

/** The player build for a browser that decodes `codecs`: the one decoding in Rust what it lacks. */
export function playerFile(codecs: string[]): string {
  return ["mp3", "aac", "flac", "vorbis", "opus"].every((c) => codecs.includes(c)) ? "player" : "player-codecs";
}

/** Which of the module's codecs decode here; `wake` is called whenever a packet has decoded. */
export async function probe(wake: () => void): Promise<string[]> {
  woken = wake;
  if (typeof AudioDecoder === "undefined") return [];
  const asks: [string, AudioDecoderConfig][] = [
    ["mp3", { codec: "mp3", sampleRate: 44100, numberOfChannels: 2 }],
    ["aac", { codec: "mp4a.40.2", sampleRate: 44100, numberOfChannels: 2 }],
    ["flac", { codec: "flac", sampleRate: 44100, numberOfChannels: 2, description: flacHead(new Uint8Array(34)) }],
    ["vorbis", { codec: "vorbis", sampleRate: 44100, numberOfChannels: 2, description: new Uint8Array(32) }],
    ["opus", { codec: "opus", sampleRate: 48000, numberOfChannels: 2 }],
  ];
  const answers = await Promise.all(asks.map(([, c]) => AudioDecoder.isConfigSupported(c).then((s) => !!s.supported, () => false)));
  supported = new Set(asks.filter((_, k) => answers[k]).map(([name]) => name));
  return [...supported];
}

/** FLAC's description: the stream marker and STREAMINFO as its only, last metadata block. */
function flacHead(streaminfo: Uint8Array): Uint8Array {
  if (streaminfo[0] === 0x66 && streaminfo[1] === 0x4c) return streaminfo;
  const d = new Uint8Array(8 + streaminfo.length);
  d.set([0x66, 0x4c, 0x61, 0x43, 0x80, 0, 0, streaminfo.length]);
  d.set(streaminfo, 8);
  return d;
}

/** Vorbis's description: identification, comment and setup headers, Xiph laced. The container gives
 * identification and setup; the comment header is made empty. */
function vorbisHead(config: Uint8Array): Uint8Array {
  const ident = config.subarray(0, 30), setup = config.subarray(30);
  const comment = new Uint8Array([3, 0x76, 0x6f, 0x72, 0x62, 0x69, 0x73, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
  const d = new Uint8Array(3 + ident.length + comment.length + setup.length);
  d.set([2, ident.length, comment.length]);
  d.set(ident, 3);
  d.set(comment, 3 + ident.length);
  d.set(setup, 3 + ident.length + comment.length);
  return d;
}

/** AAC without a config (ADTS): an AudioSpecificConfig for LC at the stated rate and channels. */
function aacConfig(rate: number, channels: number): Uint8Array {
  const rates = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
  const f = Math.max(0, rates.indexOf(rate));
  return new Uint8Array([(2 << 3) | (f >> 1), ((f & 1) << 7) | (channels << 3)]);
}

/** Output stamps within this much (µs) of a packet's are its. */
const NEAR_US = 1000;

export class Decoding {
  private decoder: AudioDecoder;
  private config: AudioDecoderConfig;
  /** The stamps of packets fed and not answered, oldest first. */
  private pending: number[] = [];
  /** Packets answered and not taken, oldest first. */
  private answered: { samples: Float32Array; channels: number; rate: number }[] = [];
  private broken = false;

  /** A decoder for `codec`, or null when it does not decode here. */
  static make(codec: string, rate: number, channels: number, config?: Uint8Array): Decoding | null {
    if (!supported.has(codec)) return null;
    let description: Uint8Array | undefined = config && config.length > 0 ? config.slice() : undefined;
    if (codec === "flac" && description) description = flacHead(description);
    if (codec === "vorbis" && description) description = vorbisHead(description);
    if (codec === "aac" && !description) description = aacConfig(rate, channels);
    try {
      return new Decoding({ codec: codecString(codec, description), sampleRate: rate, numberOfChannels: channels, description });
    } catch {
      return null;
    }
  }

  /** A decoder over `player-codecs.wasm`'s Rust decoders, for a codec only that module has. */
  static rust(codec: string, rate: number, channels: number, config?: Uint8Array): RustDecoding {
    return new RustDecoding(codec, rate, channels, config);
  }

  private constructor(config: AudioDecoderConfig) {
    this.config = config;
    this.decoder = this.made();
  }

  private made(): AudioDecoder {
    const d = new AudioDecoder({
      output: (a) => this.decoded(a),
      error: (e) => {
        console.warn(`the browser could not decode ${this.config.codec}: ${e.message}`);
        this.broken = true;
        woken();
      },
    });
    d.configure(this.config);
    return d;
  }

  /** One output, interleaved, as the oldest packet's: packets fed before the one whose stamp it bears
   * gave nothing. */
  private decoded(a: AudioData) {
    const t = a.timestamp;
    const channels = a.numberOfChannels, frames = a.numberOfFrames, rate = a.sampleRate;
    const samples = new Float32Array(frames * channels);
    const plane = new Float32Array(frames);
    for (let c = 0; c < channels; c++) {
      a.copyTo(plane, { planeIndex: c, format: "f32-planar" });
      for (let f = 0; f < frames; f++) samples[f * channels + c] = plane[f];
    }
    a.close();
    while (this.pending.length > 1 && this.pending[1] <= t + NEAR_US) {
      this.pending.shift();
      this.answered.push({ samples: new Float32Array(0), channels, rate });
    }
    this.pending.shift();
    this.answered.push({ samples, channels, rate });
    woken();
  }

  feed(unit: Uint8Array, atUs: number) {
    if (this.broken) return;
    try {
      this.decoder.decode(new EncodedAudioChunk({ type: "key", timestamp: atUs, data: unit }));
      this.pending.push(atUs);
    } catch (e) {
      console.warn(`the browser could not decode ${this.config.codec}: ${e}`);
      this.broken = true;
    }
  }

  /** The oldest packet fed is decoded, or the decoder broke. */
  ready(): boolean {
    return this.broken || this.answered.length > 0;
  }

  /** Interleaved samples of the oldest packet (-2 broken); 0 for a packet that gave none. Call only
   * when ready. */
  samples(): number {
    return this.answered.length > 0 ? this.answered[0].samples.length : -2;
  }

  channels(): number {
    return this.answered[0]?.channels ?? 0;
  }

  rate(): number {
    return this.answered[0]?.rate ?? 0;
  }

  /** Moves the oldest packet's samples into `out` (as long as `samples` said). */
  take(out: Float32Array) {
    const u = this.answered.shift();
    if (u) out.set(u.samples);
  }

  /** No more packets: what was fed decodes to its end; packets still unanswered then gave nothing. */
  finish() {
    this.decoder.flush().then(
      () => {
        for (const _ of this.pending) this.answered.push({ samples: new Float32Array(0), channels: 0, rate: 0 });
        this.pending = [];
        woken();
      },
      () => {},
    );
  }

  /** A seek: what was fed is dropped and the decoder starts afresh. */
  reset() {
    this.free();
    this.pending = [];
    this.answered = [];
    this.broken = false;
    this.decoder = this.made();
  }

  free() {
    try {
      this.decoder.close();
    } catch {}
  }
}

/** `player-codecs.wasm`'s decoders, as the module exports them. */
type Codecs = {
  decode(unit: Uint8Array): Float32Array | undefined;
  channels(): number;
  rate(): number;
  reset(): void;
  free(): void;
};
type CodecsModule = { default: () => Promise<unknown>; Codecs: { make(codec: string, rate: number, channels: number, config?: Uint8Array): Codecs | undefined } };

/** `player-codecs.wasm`, loaded beside the player module the first time a song needs it, then kept. */
let codecs: Promise<CodecsModule> | undefined;
/** Set once loaded, so packets decode as they are fed. */
let loaded: CodecsModule | undefined;

function codecsModule(): Promise<CodecsModule> {
  codecs ??= (async () => {
    const t0 = performance.now();
    // dist/pkg beside this file (dist/decoder.js).
    const here = import.meta.url;
    const m = (await import(/* @vite-ignore */ new URL("pkg/player-codecs.js", here).href)) as unknown as CodecsModule;
    await m.default();
    loaded = m;
    console.info(`the Rust decoders loaded in ${Math.round(performance.now() - t0)} ms`);
    return m;
  })();
  return codecs;
}

/** The same answers as `Decoding`, from the Rust decoders: each packet decodes as it is fed, once the
 * module is there; until then the packets wait. */
export class RustDecoding {
  private decoder?: Codecs;
  private waiting: Uint8Array[] = [];
  private answered: { samples: Float32Array; channels: number; rate: number }[] = [];
  private broken = false;
  private freed = false;

  constructor(
    private codec: string,
    private rateHz: number,
    private channelCount: number,
    private config?: Uint8Array,
  ) {
    if (loaded) this.make(loaded);
    else
      codecsModule().then(
        (m) => {
          this.make(m);
          const units = this.waiting;
          this.waiting = [];
          for (const u of units) this.feed(u);
          woken();
        },
        (e) => {
          console.warn(`the Rust decoders did not load: ${e}`);
          this.broken = true;
          woken();
        },
      );
  }

  private make(m: CodecsModule) {
    if (this.freed) return;
    this.decoder = m.Codecs.make(this.codec, this.rateHz, this.channelCount, this.config);
    if (!this.decoder) this.broken = true;
  }

  feed(unit: Uint8Array, _atUs?: number) {
    if (this.broken) return;
    if (!this.decoder) {
      this.waiting.push(unit.slice());
      return;
    }
    const samples = this.decoder.decode(unit);
    if (!samples) {
      this.broken = true;
      return;
    }
    this.answered.push({ samples, channels: this.decoder.channels(), rate: this.decoder.rate() });
  }

  ready(): boolean {
    return this.broken || this.answered.length > 0;
  }

  samples(): number {
    return this.answered.length > 0 ? this.answered[0].samples.length : -2;
  }

  channels(): number {
    return this.answered[0]?.channels ?? 0;
  }

  rate(): number {
    return this.answered[0]?.rate ?? 0;
  }

  take(out: Float32Array) {
    const u = this.answered.shift();
    if (u) out.set(u.samples);
  }

  /** Every packet fed is answered as it is fed (or once the module is there). */
  finish() {}

  reset() {
    this.waiting = [];
    this.answered = [];
    this.decoder?.reset();
  }

  free() {
    this.freed = true;
    this.decoder?.free();
    this.decoder = undefined;
  }
}
