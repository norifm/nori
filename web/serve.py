#!/usr/bin/env python3
"""Serves web/dist and passes /rest/ on to a Subsonic server, so the page and the API share one origin
(no CORS). For development only: python3 web/serve.py [--port 8080] [--server http://localhost:4533]"""
import argparse
import http.server
import urllib.error
import urllib.request
from pathlib import Path

DIST = Path(__file__).resolve().parent / "dist"
TYPES = {".js": "text/javascript", ".wasm": "application/wasm", ".css": "text/css", ".html": "text/html"}


def handler(server):
    class Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(DIST), **kwargs)

        def guess_type(self, path):
            return TYPES.get(Path(path).suffix, super().guess_type(path))

        def do_GET(self):
            if self.path.startswith("/rest/"):
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
            data = answer.read()
            self.send_response(answer.status)
            for k, v in answer.headers.items():
                if k.lower() in ("content-type", "content-range", "accept-ranges", "cache-control", "last-modified", "etag"):
                    self.send_header(k, v)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *args):
            pass

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8080)
    parser.add_argument("--server", default="http://localhost:4533")
    a = parser.parse_args()
    print(f"http://localhost:{a.port} (API from {a.server})", flush=True)
    http.server.ThreadingHTTPServer(("127.0.0.1", a.port), handler(a.server.rstrip("/"))).serve_forever()


if __name__ == "__main__":
    main()
