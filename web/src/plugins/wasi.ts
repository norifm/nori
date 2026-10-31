// The WASI a plugin's component imports, as far as a browser's plugin uses it: clocks and a stderr for its
// panics. No files, no sockets, no environment. Runs in the plugin's worker and in the sound card's worklet.

class Pollable {
  block() {}
  ready() {
    return true;
  }
}

class WasiError {
  toDebugString() {
    return "error";
  }
}

class InputStream {
  read(): Uint8Array {
    throw { tag: "closed" };
  }
  blockingRead(): Uint8Array {
    throw { tag: "closed" };
  }
  subscribe() {
    return new Pollable();
  }
}

class OutputStream {
  constructor(private line: (words: string) => void) {}
  checkWrite() {
    return 65536n;
  }
  write(bytes: Uint8Array) {
    this.line(String.fromCharCode(...bytes.subarray(0, 512)));
  }
  blockingWriteAndFlush(bytes: Uint8Array) {
    this.write(bytes);
  }
  flush() {}
  blockingFlush() {}
  subscribe() {
    return new Pollable();
  }
}

class TerminalInput {}
class TerminalOutput {}

/** The imports by interface; what a plugin writes to stderr goes to `line`. */
export function wasi(line: (words: string) => void): Record<string, Record<string, unknown>> {
  const out = new OutputStream(line);
  const nanos = () => BigInt(Math.round(Date.now() * 1e6));
  return {
    "wasi:cli/environment": { getEnvironment: () => [] },
    "wasi:cli/exit": {
      exit: () => {
        throw new Error("the plugin exited");
      },
    },
    "wasi:cli/stderr": { getStderr: () => out },
    "wasi:cli/stdin": { getStdin: () => new InputStream() },
    "wasi:cli/stdout": { getStdout: () => out },
    "wasi:cli/terminal-input": { TerminalInput },
    "wasi:cli/terminal-output": { TerminalOutput },
    "wasi:cli/terminal-stderr": { getTerminalStderr: () => undefined },
    "wasi:cli/terminal-stdin": { getTerminalStdin: () => undefined },
    "wasi:cli/terminal-stdout": { getTerminalStdout: () => undefined },
    "wasi:clocks/monotonic-clock": { now: nanos, subscribeDuration: () => new Pollable() },
    "wasi:clocks/wall-clock": {
      now: () => {
        const ms = Date.now();
        return { seconds: BigInt(Math.floor(ms / 1000)), nanoseconds: (ms % 1000) * 1e6 };
      },
    },
    "wasi:io/error": { Error: WasiError },
    "wasi:io/poll": { Pollable, poll: (list: Pollable[]) => Uint32Array.from(list.keys()) },
    "wasi:io/streams": { InputStream, OutputStream },
  };
}
