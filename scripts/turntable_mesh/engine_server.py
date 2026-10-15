"""Small HTTP engine for the dense pipeline, so one front end can serve every platform.

A desktop shell can start it on localhost; a phone or a browser can talk to one
running on another machine. It starts runs, lists them, streams their event log
and serves their artifacts. Standard library only.

  GET  /api/health                       schema and capabilities
  GET  /api/settings                     every setting with group, meaning, kind, default
  GET  /api/data?path=<relative>         folders under --data; "inputs": true where a run can start
  GET  /api/runs                         known runs with status
  POST /api/runs                         start a run; JSON body, see start_run()
  GET  /api/runs/<id>/events?since=N     events from line N; {"events": [...], "next": M}
  POST /api/runs/<id>/cancel             ask a run to stop
  GET  /api/runs/<id>/files/<path>       an artifact of the run
  GET  /<anything else>                  the front end, when --static is given

Runs live in --runs. Inputs must lie under --data, so a client cannot point the
engine at arbitrary paths. It binds to 127.0.0.1 unless told otherwise; when
bound elsewhere a --token is required and clients send "Authorization: Bearer".
This is a development-grade server: no TLS, no accounts.
"""

import argparse
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import mimetypes
import os
from pathlib import Path
import re
import subprocess
import sys
import time
from urllib.parse import parse_qs, unquote, urlparse

from .dense_config import build, settings_schema
from .dense_events import SCHEMA, read

REPOSITORY = Path(__file__).resolve().parents[2]


def inside(root, candidate):
    """Resolved ``candidate`` if it lies under ``root``, else None."""
    root, candidate = Path(root).resolve(), Path(candidate).resolve()
    return candidate if candidate == root or root in candidate.parents else None


def run_status(folder):
    events = read(folder / "events.jsonl")
    status, started, stage, fraction = "unknown", None, None, 0.0
    for event in events:
        if event["type"] == "run_started":
            status, started = "running", event["time"]
        elif event["type"] == "stage_started":
            stage, fraction = event["stage"], 0.0
        elif event["type"] == "progress":
            fraction = event["fraction"]
        elif event["type"] == "run_finished":
            status = event["status"]
    if status != "running":
        stage, fraction = None, 0.0
    return {"id": folder.name, "status": status, "started": started, "stage": stage, "stage_fraction": fraction,
            "events": len(events)}


class Engine:
    def __init__(self, runs, data, python=None, torch_python=None, device="cpu"):
        self.runs, self.data = Path(runs).resolve(), Path(data).resolve()
        self.runs.mkdir(parents=True, exist_ok=True)
        self.python = python or os.environ.get("CRISPMESH_PYTHON") or sys.executable
        self.torch_python = torch_python or os.environ.get("CRISPMESH_TORCH_PYTHON") or sys.executable
        self.device = device

    def list_runs(self):
        folders = [f for f in self.runs.iterdir() if (f / "events.jsonl").is_file()]
        return sorted((run_status(f) for f in folders), key=lambda r: r["started"] or 0, reverse=True)

    def run_folder(self, run_id):
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,80}", run_id):
            return None
        folder = self.runs / run_id
        return folder if (folder / "events.jsonl").is_file() else None

    def list_data(self, relative):
        """Folders under the data directory, flagged when they look like dense inputs."""
        folder = inside(self.data, self.data / relative)
        if folder is None or not folder.is_dir():
            raise ValueError("no such folder under the data directory")
        entries = []
        for child in sorted(folder.iterdir()):
            if child.name.startswith("."):
                continue
            photos = child.is_dir() and sum(1 for f in child.iterdir() if f.suffix.lower() in (".png", ".jpg", ".jpeg")) >= 3
            entries.append({"name": child.name, "directory": child.is_dir(),
                            "inputs": child.is_dir() and (child / "cameras.json").is_file(), "photos": photos,
                            "calibration": child.is_file() and child.suffix == ".json"
                            and '"crispmesh_lens_calibration_v1"' in child.read_text(errors="ignore")[:400]})
        return {"path": str(folder.relative_to(self.data)).replace(os.sep, "/").strip("."), "entries": entries[:500]}

    def start_run(self, body):
        if not isinstance(body, dict):
            raise ValueError("JSON object required")
        """Body: {"photos", "calibration"}, or {"inputs": path}, or {"scene", "prepared", "raw_masks"}; optional "name",
        "device", "settings" {key: value}, "reference" path. Paths are relative to --data."""
        name = re.sub(r"[^A-Za-z0-9._-]+", "-", str(body.get("name") or "run")).strip("-.")[:40] or "run"
        run_id = f"{time.strftime('%Y%m%d-%H%M%S')}-{name}"
        overrides = [f"{k}={','.join(map(str, v)) if isinstance(v, list) else v}" for k, v in (body.get("settings") or {}).items()]
        build(None, overrides)  # validate before anything starts
        command = [self.python, "-m", "scripts.turntable_mesh.dense_pipeline", "--output", str(self.runs / run_id),
                   "--device", str(body.get("device") or self.device), "--python", self.python,
                   "--torch-python", self.torch_python]
        keys = (("photos", "calibration") if body.get("photos") else ("inputs",) if body.get("inputs")
                else ("scene", "prepared", "raw_masks"))
        for key in (*keys, *(("reference",) if body.get("reference") else ())):
            path = inside(self.data, self.data / str(body.get(key) or ""))
            if path is None or not path.exists():
                raise ValueError(f"{key} must be an existing path under the data directory")
            command += ["--" + key.replace("_", "-"), str(path)]
        for item in overrides:
            command += ["--set", item]
        with open(self.runs / f"{run_id}.driver.log", "w") as log:  # the child keeps its own handle
            subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, cwd=REPOSITORY,
                             env={**os.environ, "PYTHONPATH": str(REPOSITORY)}, start_new_session=os.name == "posix")
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not (self.runs / run_id / "events.jsonl").is_file():
            time.sleep(0.1)
        if not (self.runs / run_id / "events.jsonl").is_file():
            raise RuntimeError("run did not start: " + (self.runs / f"{run_id}.driver.log").read_text()[-500:])
        return {"id": run_id}


def handler_for(engine, static, token):
    class Handler(BaseHTTPRequestHandler):
        server_version = "crispmesh-engine/1"

        def log_message(self, *args):
            pass

        head = False

        def reply(self, payload, status=HTTPStatus.OK):
            data = json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            if not self.head:
                self.wfile.write(data)

        def send_file(self, path):
            data = path.read_bytes()
            self.send_response(HTTPStatus.OK)
            self.send_header("Content-Type", mimetypes.guess_type(path.name)[0] or "application/octet-stream")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            if not self.head:
                self.wfile.write(data)

        def authorised(self):
            return not token or self.headers.get("Authorization") == "Bearer " + token

        def do_OPTIONS(self):
            self.send_response(HTTPStatus.NO_CONTENT)
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Headers", "Authorization, Content-Type")
            self.send_header("Access-Control-Allow-Methods", "GET, HEAD, POST, OPTIONS")
            self.end_headers()

        def route(self, method):
            url = urlparse(self.path)
            parts = [unquote(p) for p in url.path.split("/") if p]
            if parts[:1] != ["api"]:
                if method == "GET" and static:
                    target = inside(static, Path(static) / "/".join(parts)) if parts else Path(static) / "index.html"
                    if target and target.is_dir():
                        target = target / "index.html"
                    if target and target.is_file():
                        return self.send_file(target)
                return self.reply({"error": "not found"}, HTTPStatus.NOT_FOUND)
            if not self.authorised():
                return self.reply({"error": "unauthorised"}, HTTPStatus.UNAUTHORIZED)
            if method == "GET" and parts == ["api", "health"]:
                return self.reply({"schema": SCHEMA, "device": engine.device, "can_start_runs": True})
            if method == "GET" and parts == ["api", "data"]:
                try:
                    return self.reply(engine.list_data(parse_qs(url.query).get("path", [""])[0]))
                except ValueError as problem:
                    return self.reply({"error": str(problem)}, HTTPStatus.BAD_REQUEST)
            if method == "GET" and parts == ["api", "settings"]:
                return self.reply({"settings": settings_schema()})
            if parts == ["api", "runs"]:
                if method == "GET":
                    return self.reply({"runs": engine.list_runs()})
                length = int(self.headers.get("Content-Length") or 0)
                if not 0 < length < 1_000_000:
                    return self.reply({"error": "JSON body required"}, HTTPStatus.BAD_REQUEST)
                try:
                    return self.reply(engine.start_run(json.loads(self.rfile.read(length))), HTTPStatus.CREATED)
                except (ValueError, RuntimeError) as problem:  # includes malformed JSON
                    return self.reply({"error": str(problem)}, HTTPStatus.BAD_REQUEST)
            if len(parts) >= 4 and parts[:2] == ["api", "runs"]:
                folder = engine.run_folder(parts[2])
                if folder is None:
                    return self.reply({"error": "unknown run"}, HTTPStatus.NOT_FOUND)
                if method == "GET" and parts[3] == "events":
                    try:
                        since = max(0, int(parse_qs(url.query).get("since", ["0"])[0]))
                    except ValueError:
                        return self.reply({"error": "since must be a number"}, HTTPStatus.BAD_REQUEST)
                    events = read(folder / "events.jsonl", since)
                    return self.reply({"events": events, "next": events[-1]["seq"] + 1 if events else since})
                if method == "POST" and parts[3] == "cancel":
                    (folder / "cancel").touch()
                    return self.reply({"id": folder.name, "cancel_requested": True})
                if method == "GET" and parts[3] == "files" and len(parts) > 4:
                    target = inside(folder, folder / "/".join(parts[4:]))
                    if target and target.is_file():
                        return self.send_file(target)
            return self.reply({"error": "not found"}, HTTPStatus.NOT_FOUND)

        def do_GET(self):
            self.route("GET")

        def do_HEAD(self):
            self.head = True
            self.route("GET")

        def do_POST(self):
            self.route("POST")

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--runs", type=Path, required=True, help="directory holding one folder per run")
    parser.add_argument("--data", type=Path, required=True, help="directory under which run inputs must lie")
    parser.add_argument("--static", type=Path, help="built front end to serve at /")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--token", help="required when not bound to localhost")
    parser.add_argument("--device", choices=("mps", "cuda", "cpu"), default="cpu", help="default device of new runs")
    parser.add_argument("--python")
    parser.add_argument("--torch-python")
    args = parser.parse_args()
    if args.host not in ("127.0.0.1", "localhost", "::1") and not args.token:
        parser.error("--token is required when binding to a non-local address")
    engine = Engine(args.runs, args.data, args.python, args.torch_python, args.device)
    server = ThreadingHTTPServer((args.host, args.port), handler_for(engine, args.static, args.token))
    print(f"crispmesh engine on http://{args.host}:{args.port}  runs={engine.runs}  data={engine.data}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
