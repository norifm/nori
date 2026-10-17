// The parts of WebCodecs' audio decoding the player uses (TypeScript's DOM library has the video side
// only).

interface AudioDecoderConfig {
  codec: string;
  sampleRate: number;
  numberOfChannels: number;
  description?: BufferSource;
}

interface AudioData {
  readonly numberOfFrames: number;
  readonly numberOfChannels: number;
  readonly sampleRate: number;
  readonly timestamp: number;
  copyTo(destination: BufferSource, options: { planeIndex: number; format?: "f32-planar" | "f32" }): void;
  close(): void;
}

declare class EncodedAudioChunk {
  constructor(init: { type: "key" | "delta"; timestamp: number; data: BufferSource });
}

declare class AudioDecoder {
  constructor(init: { output: (data: AudioData) => void; error: (e: DOMException) => void });
  static isConfigSupported(config: AudioDecoderConfig): Promise<{ supported?: boolean }>;
  configure(config: AudioDecoderConfig): void;
  decode(chunk: EncodedAudioChunk): void;
  flush(): Promise<void>;
  close(): void;
}
