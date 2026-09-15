"""33. An HTTP endpoint that validates an uploaded ``.fec`` (standard library only).

Shows: ``http.server`` handling ``POST /validate`` with the raw ``.fec``
bytes as the request body and answering with JSON: the verdict, every
finding, and the reconciliation summary. No framework; the same handler
logic drops into Flask or FastAPI unchanged because it is a function of
``bytes`` (see example 32).

Try it::

    python examples/33_http_validation_server.py --serve 8765
    curl -s --data-binary @tests/fixtures/F3XN_2011831.fec http://127.0.0.1:8765/validate

Without ``--serve`` the example runs a self-test: it starts the server on
a free port in a background thread, POSTs the given file to it with
``urllib``, prints the response, and stops.

Run:
    python examples/33_http_validation_server.py [path/to/filing.fec]
    python examples/33_http_validation_server.py --serve [port]
"""

from __future__ import annotations

import json
import sys
import threading
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "invalid" / "duplicate_tran_id.fec"
MAX_BODY = 512 * 1024 * 1024  # the largest filings are a few hundred MB


def validate_bytes(data: bytes) -> tuple[int, dict[str, Any]]:
    """(HTTP status, JSON body). 200 acceptable, 422 not, 400 unparseable."""
    try:
        filing = hardmoney.parse(data, lenient=True)
    except hardmoney.FecError as e:
        return 400, {"ok": False, "error": str(e), "line": e.line_no}
    v = filing.validate()
    body: dict[str, Any] = {
        "ok": v.is_acceptable,
        "form_type": filing.form_type,
        "version": filing.version,
        "lines": len(filing.lines),
        "findings": [
            {"severity": f.severity, "rule": f.rule, "line": f.line_no,
             "record": f.form_type, "field": f.field, "message": f.message}
            for f in v
        ],
    }
    try:
        r = filing.reconcile()
        body["reconciliation"] = {
            "balances": r.balances,
            "mismatches": [{"column": c.column, "line": c.line, "delta": str(c.delta)} for c in r.mismatches()],
        }
    except hardmoney.UnsupportedForm:
        body["reconciliation"] = None
    return (200 if v.is_acceptable else 422), body


class Handler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:  # noqa: N802 (http.server's naming)
        if self.path != "/validate":
            self.send_error(404, "POST /validate")
            return
        length = int(self.headers.get("Content-Length", "0"))
        if length <= 0 or length > MAX_BODY:
            self.send_error(411 if length <= 0 else 413)
            return
        status, body = validate_bytes(self.rfile.read(length))
        payload = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A002
        sys.stderr.write("  server: " + format % args + "\n")


def serve(port: int) -> None:
    with ThreadingHTTPServer(("127.0.0.1", port), Handler) as httpd:
        print(f"POST a .fec to http://127.0.0.1:{httpd.server_address[1]}/validate (Ctrl-C to stop)")
        httpd.serve_forever()


def self_test(path: Path) -> int:
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)  # port 0: any free port
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    url = f"http://127.0.0.1:{httpd.server_address[1]}/validate"
    print(f"POST {path.name} -> {url}", flush=True)
    req = urllib.request.Request(url, data=path.read_bytes(), method="POST",
                                 headers={"Content-Type": "application/octet-stream"})
    try:
        with urllib.request.urlopen(req) as resp:
            status, body = resp.status, json.loads(resp.read())
    except urllib.error.HTTPError as e:  # 4xx still carries the JSON body
        status, body = e.code, json.loads(e.read())
    finally:
        httpd.shutdown()
        httpd.server_close()
    print(f"HTTP {status}")
    print(json.dumps(body, indent=2))
    return 0


def main(argv: Sequence[str]) -> int:
    if argv and argv[0] == "--serve":
        serve(int(argv[1]) if len(argv) > 1 else 8765)
        return 0
    return self_test(Path(argv[0]) if argv else DEFAULT)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
