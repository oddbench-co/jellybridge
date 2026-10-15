#!/usr/bin/env python3
"""Serves the test payload to VMs and takes their uploaded results.

    serve.py WWW_DIR [PORT]

Guests reach it at http://10.0.2.2:PORT (QEMU user networking maps that to
the host's loopback, so nothing listens on the LAN). GET serves WWW_DIR;
PUT /results/<path> stores a file under WWW_DIR/results/.
"""
import http.server, os, sys

ROOT = os.path.abspath(sys.argv[1])
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8765


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=ROOT, **kw)

    def do_PUT(self):
        rel = os.path.normpath(self.path.lstrip("/"))
        if not rel.startswith("results/") or ".." in rel.split(os.sep):
            self.send_error(403)
            return
        dest = os.path.join(ROOT, rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        length = int(self.headers.get("Content-Length", 0))
        with open(dest, "wb") as f:
            remaining = length
            while remaining > 0:
                chunk = self.rfile.read(min(remaining, 1 << 20))
                if not chunk:
                    break
                f.write(chunk)
                remaining -= len(chunk)
        self.send_response(201)
        self.end_headers()

    def log_message(self, fmt, *args):
        sys.stderr.write("%s %s\n" % (self.command, self.path))


http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
