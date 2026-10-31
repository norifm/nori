#!/usr/bin/env python3
"""Serves web/dist and passes /rest/ on to a Subsonic server, so the page and the API share one origin
(no CORS). For development only: python3 web/serve.py [--port 8080] [--server http://localhost:4533]
[--log] [--kbps N] [--files-kbps N]. --log prints each API request (endpoint, range, status); --kbps
sends the API's answers (songs) at most that fast, --files-kbps the client's own files, as a slow network
would."""
import argparse
import http.server
import time
import urllib.error
import urllib.request
from pathlib import Path

DIST = Path(__file__).resolve().parent / "dist"
TYPES = {".js": "text/javascript", ".wasm": "application/wasm", ".css": "text/css", ".html": "text/html"}


def handler(server, log, kbps, files_kbps):
    class Handler(http.server.SimpleHTTPRequestHandler):
        def end_headers(self):
            # A new build is seen at the next load.
            self.send_header("Cache-Control", "no-cache")
            super().end_headers()

        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(DIST), **kwargs)

        def copyfile(self, source, outputfile):
            self.paced(source.read, outputfile.write, files_kbps)

        def paced(self, read, write, kbps):
            started, sent = time.monotonic(), 0
            try:
                while chunk := read(16 * 1024):
                    write(chunk)
                    sent += len(chunk)
                    if kbps:
                        time.sleep(max(0.0, started + sent * 8 / (kbps * 1000) - time.monotonic()))
            except (BrokenPipeError, ConnectionResetError):
                pass

        def guess_type(self, path):
            return TYPES.get(Path(path).suffix, super().guess_type(path))

        def do_GET(self):
            if self.path.startswith(("/rest/", "/nori/")):
                return self.forward()
            super().do_GET()

        def do_POST(self):
            self.forward()

        def forward(self):
            length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(length) if length else None
            keep = {k: v for k, v in self.headers.items() if k.lower() in ("range", "content-type", "accept")}
            request = urllib.request.Request(server + self.path, data=body, headers=keep, method=self.command)
            try:
                answer = urllib.request.urlopen(request, timeout=60)
            except urllib.error.HTTPError as e:
                answer = e
            self.send_response(answer.status)
            for k, v in answer.headers.items():
                if k.lower() in ("content-type", "content-range", "content-length", "accept-ranges", "cache-control", "last-modified", "etag"):
                    self.send_header(k, v)
            if answer.headers.get("Content-Length") is None:
                self.send_header("Connection", "close")
                self.close_connection = True
            self.end_headers()
            if log:
                print(self.command, self.path.split("?")[0], self.headers.get("Range") or "", answer.status, flush=True)
            # Passed on as it comes, so a song streams.
            self.paced(answer.read1, self.wfile.write, kbps)

        def log_message(self, *args):
            pass

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8080)
    parser.add_argument("--server", default="http://localhost:4533")
    parser.add_argument("--log", action="store_true")
    parser.add_argument("--kbps", type=int, default=0)
    parser.add_argument("--files-kbps", type=int, default=0)
    a = parser.parse_args()
    print(f"http://localhost:{a.port} (API from {a.server})", flush=True)
    http.server.ThreadingHTTPServer(("127.0.0.1", a.port), handler(a.server.rstrip("/"), a.log, a.kbps, a.files_kbps)).serve_forever()


if __name__ == "__main__":
    main()
