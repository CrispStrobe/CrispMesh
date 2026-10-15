"""Checks for the HTTP engine against a recorded run (standard library only).

The recorded run is tests/fixtures/dense-run-sphere, copied into a temporary
runs directory; no pipeline is started. Set CRISPMESH_TEST_ENGINE_RUN=1 to also
start one real run on the synthetic sphere through POST /api/runs (needs the
pipeline requirements, about a minute on CPU).
"""

from dataclasses import fields
import http.client
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest

from scripts.turntable_mesh.dense_config import DenseConfig
from scripts.turntable_mesh.dense_events import SCHEMA
from scripts.turntable_mesh.engine_server import Engine, ThreadingHTTPServer, handler_for

REPOSITORY = Path(__file__).resolve().parents[2]
FIXTURE = REPOSITORY / "tests/fixtures/dense-run-sphere"
RUN = "sphere-replay"


class Served:
    """An engine on a free local port, serving in a background thread."""

    def __init__(self, root, static=None, token=None):
        self.engine = Engine(root / "runs", root / "data")
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), handler_for(self.engine, static, token))
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def request(self, method, path, body=None, headers=None):
        """Status, headers and body. The path is sent exactly as given (no client-side normalisation)."""
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        try:
            payload = None if body is None else json.dumps(body).encode()
            connection.request(method, path, body=payload, headers=headers or {})
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            connection.close()

    def json(self, path, **kwargs):
        status, _, data = self.request(kwargs.pop("method", "GET"), path, **kwargs)
        return status, json.loads(data)

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=10)


class EngineReplayTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        cls.root = Path(cls.folder.name).resolve()
        (cls.root / "data").mkdir()
        shutil.copytree(FIXTURE, cls.root / "runs" / RUN)
        (cls.root / "runs" / "not-a-run").mkdir()  # no events.jsonl: must not be listed
        (cls.root / "secret.txt").write_text("outside the runs directory")
        (cls.root / "runs" / "secret.txt").write_text("outside the run folder")
        (cls.root / "static").mkdir()
        (cls.root / "static" / "index.html").write_text("<!doctype html><title>front end</title>")
        cls.lines = (FIXTURE / "events.jsonl").read_text(encoding="utf-8").splitlines()
        cls.served = Served(cls.root, static=cls.root / "static")

    @classmethod
    def tearDownClass(cls):
        cls.served.close()
        cls.folder.cleanup()

    def test_health(self):
        status, body = self.served.json("/api/health")
        self.assertEqual(status, 200)
        self.assertEqual(body["schema"], SCHEMA)
        self.assertEqual(body["device"], "cpu")
        self.assertIs(body["can_start_runs"], True)

    def test_settings_cover_every_field(self):
        status, body = self.served.json("/api/settings")
        self.assertEqual(status, 200)
        self.assertEqual([row["name"] for row in body["settings"]], [f.name for f in fields(DenseConfig)])
        for row in body["settings"]:
            self.assertEqual(set(row), {"name", "group", "meaning", "kind", "default"})
            self.assertIn(row["kind"], ("boolean", "integer", "number", "integer_list", "number_list"))

    def test_runs_lists_the_recorded_run_only(self):
        status, body = self.served.json("/api/runs")
        self.assertEqual(status, 200)
        self.assertEqual([run["id"] for run in body["runs"]], [RUN])
        run = body["runs"][0]
        self.assertEqual(run["status"], "complete")
        self.assertEqual(run["events"], len(self.lines))
        self.assertEqual(run["started"], json.loads(self.lines[0])["time"])

    def test_events_paging(self):
        total = len(self.lines)
        self.assertGreater(total, 10)
        status, everything = self.served.json(f"/api/runs/{RUN}/events")
        self.assertEqual(status, 200)
        self.assertEqual([e["seq"] for e in everything["events"]], list(range(total)))
        self.assertEqual(everything["next"], total)
        self.assertEqual(everything["events"][0]["type"], "run_started")
        self.assertEqual(everything["events"][-1]["type"], "run_finished")
        # Walk the log in pages the way a polling client does.
        position, collected = 0, []
        for stop in (7, total - 3, total):
            _, page = self.served.json(f"/api/runs/{RUN}/events?since={position}")
            fresh = [e for e in page["events"] if e["seq"] < stop]
            self.assertEqual([e["seq"] for e in page["events"]], list(range(position, total)))
            collected += fresh
            position = stop
        self.assertEqual(collected, everything["events"])
        _, tail = self.served.json(f"/api/runs/{RUN}/events?since={total - 4}")
        self.assertEqual([e["seq"] for e in tail["events"]], list(range(total - 4, total)))
        self.assertEqual(tail["next"], total)
        _, empty = self.served.json(f"/api/runs/{RUN}/events?since={total}")
        self.assertEqual(empty, {"events": [], "next": total})

    def test_artifacts_named_by_events_are_served(self):
        _, body = self.served.json(f"/api/runs/{RUN}/events")
        paths = sorted({e["path"] for e in body["events"] if e["type"] == "artifact" and e["kind"] != "preview_volume"})
        self.assertIn("mesh/mesh.stl", paths)
        for path in paths:
            status, headers, data = self.served.request("GET", f"/api/runs/{RUN}/files/{path}")
            self.assertEqual(status, 200, path)
            self.assertEqual(data, (FIXTURE / path).read_bytes(), path)
            self.assertEqual(int(headers["Content-Length"]), len(data))
        status, headers, _ = self.served.request("GET", f"/api/runs/{RUN}/files/check/preview.png")
        self.assertEqual((status, headers["Content-Type"]), (200, "image/png"))

    def test_missing_things_are_404(self):
        for path in (f"/api/runs/{RUN}/files/no-such-file", f"/api/runs/{RUN}/files/mesh", f"/api/runs/{RUN}/files",
                     "/api/runs/unknown/events", "/api/runs/not-a-run/events", "/api/nothing", f"/api/runs/{RUN}"):
            status, body = self.served.json(path)
            self.assertEqual(status, 404, path)
            self.assertIn("error", body)

    def test_path_traversal_is_rejected(self):
        escapes = (f"/api/runs/{RUN}/files/../secret.txt",
                   f"/api/runs/{RUN}/files/../../secret.txt",
                   f"/api/runs/{RUN}/files/mesh/../../secret.txt",
                   f"/api/runs/{RUN}/files/%2e%2e/secret.txt",
                   f"/api/runs/{RUN}/files/..%2fsecret.txt",
                   f"/api/runs/{RUN}/files/..%2f..%2fsecret.txt",
                   f"/api/runs/{RUN}/files/..%5csecret.txt",
                   f"/api/runs/{RUN}/files/%2e%2e%5c%2e%2e%5csecret.txt",
                   "/api/runs/../files/secret.txt",
                   "/api/runs/..%2f/files/secret.txt",
                   f"/api/runs/..%2f{RUN}/events",
                   f"/api/runs/{RUN}%2f..%2f..%2fsecret.txt/events",
                   "/../secret.txt",
                   "/..%2fsecret.txt",
                   "/%2e%2e/%2e%2e/secret.txt")
        for path in escapes:
            status, _, data = self.served.request("GET", path)
            self.assertEqual(status, 404, path)
            self.assertNotIn(b"outside", data, path)
        absolute = (self.root / "secret.txt").as_posix()
        for path in (f"/api/runs/{RUN}/files/{absolute}", f"/api/runs/{RUN}/files//{absolute.lstrip('/')}",
                     "/" + absolute.lstrip("/")):
            status, _, data = self.served.request("GET", path)
            self.assertEqual(status, 404, path)
            self.assertNotIn(b"outside", data, path)

    def test_front_end_is_served_outside_api(self):
        for path in ("/", "/index.html"):
            status, headers, data = self.served.request("GET", path)
            self.assertEqual((status, headers["Content-Type"]), (200, "text/html"), path)
            self.assertIn(b"front end", data)
        self.assertEqual(self.served.request("GET", "/missing.js")[0], 404)

    def test_start_run_rejects_bad_requests_without_starting_anything(self):
        before = sorted(p.name for p in (self.root / "runs").iterdir())
        for body in ({"inputs": "does-not-exist"}, {"inputs": "../runs/" + RUN}, {"inputs": str(self.root / "runs" / RUN)},
                     {"inputs": ".", "settings": {"no_such_setting": 1}}, {"scene": "a.sfm"}):
            status, reply = self.served.json("/api/runs", method="POST", body=body)
            self.assertEqual(status, 400, body)
            self.assertIn("error", reply)
        self.assertEqual(self.served.request("POST", "/api/runs")[0], 400)
        self.assertEqual(sorted(p.name for p in (self.root / "runs").iterdir()), before)

    def test_cancel_leaves_a_marker(self):
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as folder:
            root = Path(folder).resolve()
            (root / "data").mkdir()
            shutil.copytree(FIXTURE, root / "runs" / RUN)
            served = Served(root)
            try:
                status, body = served.json(f"/api/runs/{RUN}/cancel", method="POST")
                self.assertEqual((status, body), (200, {"id": RUN, "cancel_requested": True}))
                self.assertTrue((root / "runs" / RUN / "cancel").is_file())
                self.assertEqual(served.request("GET", "/")[0], 404)  # no --static: nothing outside /api
            finally:
                served.close()


class EngineTokenTest(unittest.TestCase):
    def test_token_guards_the_api(self):
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as folder:
            root = Path(folder).resolve()
            (root / "data").mkdir()
            shutil.copytree(FIXTURE, root / "runs" / RUN)
            served = Served(root, token="s3cret")
            try:
                for path in ("/api/health", "/api/runs", f"/api/runs/{RUN}/events", f"/api/runs/{RUN}/files/mesh/result.json"):
                    self.assertEqual(served.request("GET", path)[0], 401, path)
                    self.assertEqual(served.request("GET", path, headers={"Authorization": "Bearer wrong"})[0], 401, path)
                    self.assertEqual(served.request("GET", path, headers={"Authorization": "Bearer s3cret"})[0], 200, path)
                self.assertEqual(served.request("POST", f"/api/runs/{RUN}/cancel")[0], 401)
                self.assertFalse((root / "runs" / RUN / "cancel").exists())
            finally:
                served.close()

    def test_non_local_bind_without_token_is_refused(self):
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as folder:
            for host in ("0.0.0.0", "192.0.2.1"):
                result = subprocess.run(
                    [sys.executable, "-m", "scripts.turntable_mesh.engine_server", "--runs", str(Path(folder) / "runs"),
                     "--data", folder, "--host", host, "--port", "0"],
                    cwd=REPOSITORY, env={**os.environ, "PYTHONPATH": str(REPOSITORY)}, capture_output=True, text=True,
                    timeout=60)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("--token is required", result.stderr)
                self.assertNotIn("crispmesh engine on", result.stdout)


@unittest.skipUnless(os.environ.get("CRISPMESH_TEST_ENGINE_RUN") == "1", "set CRISPMESH_TEST_ENGINE_RUN=1 to start a real run")
class EngineRealRunTest(unittest.TestCase):
    def test_started_run_completes_and_is_observable(self):
        from scripts.turntable_mesh import synthetic_scene

        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as folder:
            root = Path(folder).resolve()
            synthetic_scene.write(root / "data" / "sphere")
            served = Served(root)
            try:
                settings = dict(item.split("=") for item in synthetic_scene.SMALL)
                status, body = served.json("/api/runs", method="POST",
                                           body={"inputs": "sphere", "name": "engine smoke", "device": "cpu",
                                                 "settings": settings})
                self.assertEqual(status, 201, body)
                run = body["id"]
                position, seen, deadline = 0, [], time.monotonic() + 600
                while time.monotonic() < deadline and not any(e["type"] == "run_finished" for e in seen):
                    _, page = served.json(f"/api/runs/{run}/events?since={position}")
                    seen += page["events"]
                    position = page["next"]
                    time.sleep(0.5)
                driver_log = (root / "runs" / f"{run}.driver.log").read_text()
                finished = [e for e in seen if e["type"] == "run_finished"]
                self.assertTrue(finished, "no run_finished within the deadline\n" + driver_log[-3000:])
                self.assertEqual(finished[0]["status"], "complete", driver_log[-3000:])
                self.assertEqual([e["seq"] for e in seen], list(range(len(seen))))
                kinds = {e["kind"] for e in seen if e["type"] == "artifact"}
                self.assertIn("final_mesh", kinds)
                self.assertIn("preview_mesh", kinds, "live preview side processes produced nothing")
                for event in seen:
                    if event["type"] == "artifact" and event["kind"] in ("final_mesh", "preview_mesh", "report"):
                        status, _, data = served.request("GET", f"/api/runs/{run}/files/{event['path']}")
                        self.assertEqual(status, 200, event["path"])
                        self.assertGreater(len(data), 0)
                _, listing = served.json("/api/runs")
                self.assertEqual([(r["id"], r["status"]) for r in listing["runs"]], [(run, "complete")])
            finally:
                served.close()


if __name__ == "__main__":
    unittest.main()
