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
import shutil
import math
import threading
import sys
import time
from urllib.parse import parse_qs, unquote, urlparse

from scripts.photo_regression.stage_cache import inventory
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
    def __init__(self, runs, data, python=None, torch_python=None, device="cpu", capture_tools=None):
        self.runs, self.data = Path(runs).resolve(), Path(data).resolve()
        self.runs.mkdir(parents=True, exist_ok=True)
        self.data.mkdir(parents=True, exist_ok=True)
        self.capture_lock = threading.Lock()
        self.python = python or os.environ.get("CRISPMESH_PYTHON") or sys.executable
        self.torch_python = torch_python or os.environ.get("CRISPMESH_TORCH_PYTHON") or sys.executable
        self.device = device
        self.capture_tools = Path(capture_tools).resolve() if capture_tools else None
        self.capture_child = None

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

    def health(self):
        result = {"schema": SCHEMA, "device": self.device, "can_start_runs": True}
        if self.capture_tools:
            tools = json.loads(self.capture_tools.read_text())
            described = json.loads(subprocess.check_output([tools['native'], 'run', '--describe'], text=True))
            points = [point for point in described['start_points'] if point['id'] in {'inputs', 'photos'}]
            for point in points:
                if point['id'] == 'photos':
                    mask_choice=next(choice for choice in point['providers'] if choice['module']=='masks')
                    imported=next(option for option in mask_choice['options'] if option['id']=='import')
                    camera_choice=next(choice for choice in point['providers'] if choice['module']=='cameras')
                    padding=next(dict(option) for option in camera_choice['settings'] if option['name']=='object-crop-padding')
                    padding['default']=.15
                    turntable=next(option for option in camera_choice['options'] if option['id']=='turntable')
                    groups=next(dict(option) for option in turntable['settings'] if option['name']=='turntable-turn-sizes')
                    groups['meaning']='Photo counts per camera-height group; camera angles are recovered from the images.'
                    sam_available=all(bool(tools.get(key)) and Path(tools[key]).exists() for key in ['sam_source','sam_checkpoint'])
                    mask_choice.update(default='refined' if sam_available else 'automatic',settings=[],options=[{'id':'refined','label':'Refined foreground','meaning':'Additional SAM refinement of automatic masks; slower.','available':sam_available,'settings':[],'inputs':[]},{'id':'automatic','label':'Automatic foreground','meaning':'Track the object and recover masks automatically, or use the masks from the app preview.','available':True,'settings':[],'inputs':[]},imported])
                    point['providers']=[mask_choice]
                    point['option_groups']=[{'id':'capture','label':'Capture','settings':[padding,groups]}]
                    point['label'] = 'Object photos'
                    point['meaning'] = 'Original photos of one object; masks and cameras are recovered automatically.'
                    for field in point['fields']:
                        if field['key'] == 'photos': field['help']='Photos may use uneven angles and several camera heights.'
                        if field['key'] == 'calibration': field.update(required=False, automatic=True, help='Optional measured lens calibration; otherwise estimate focal length and distortion from the photos.')
            result['device']='Mac'
            result.update(methods=[method for method in described['methods'] if method['id']=='splats'], base_surfaces=['silhouette','stereo'], start_points=points, uploads=True, chooses_device=False, scores_reference=False, refine_runs=True)
        return result

    def capture_run(self, body):
        if self.capture_child is not None and self.capture_child.poll() is None:
            raise ValueError('a reconstruction is already running on this engine')
        if shutil.disk_usage(self.runs).free < 8.8*2**30:raise ValueError('not enough free disk space for capture processing (8.8 GiB reserve)')
        tools = json.loads(self.capture_tools.read_text())
        name = re.sub(r'[^A-Za-z0-9._-]+','-',str(body.get('name') or 'capture')).strip('-.')[:40] or 'capture'
        run_id = time.strftime('%Y%m%d-%H%M%S')+'-'+name
        output = self.runs/run_id
        if output.exists(): raise ValueError('run name already exists; retry')
        base = body.get('base_surface','silhouette')
        if base not in {'silhouette','stereo'}: raise ValueError('invalid base surface')
        key = 'photos' if body.get('photos') else 'inputs'
        source = inside(self.data,self.data/str(body.get(key) or ''))
        if source is None or not source.is_dir(): raise ValueError(key+' must be a folder under the data directory')
        if key == 'photos':
            command = [self.python,'-m','scripts.photo_regression.capture_pipeline','--photos',str(source),'--output',str(output),'--tools',str(self.capture_tools),'--base-surface',base,'--data-root',str(self.data)]
            foreground=tools.get('foreground_provider','app')
            if foreground not in {'app','independent'}:raise ValueError('invalid host foreground provider')
            command+=['--foreground-provider',foreground]
            camera_search=tools.get('camera_search','hybrid')
            if camera_search not in {'hybrid','tight','wide'}:raise ValueError('invalid capture camera search path')
            command+=['--camera-search',camera_search]
            providers=body.get('providers',{})
            default_masks='refined' if all(bool(tools.get(key)) and Path(tools[key]).exists() for key in ['sam_source','sam_checkpoint']) else 'automatic'
            if not isinstance(providers,dict) or set(providers)-{'masks'} or providers.get('masks',default_masks) not in {'automatic','refined','import'}:raise ValueError('invalid capture mask provider')
            if providers.get('masks',default_masks)=='refined':command+=['--refine-masks']
            if providers.get('masks',default_masks)=='automatic':command+=['--no-refine-masks']
            if providers.get('masks')=='import':
                masks=inside(self.data,self.data/str(body.get('masks_import') or ''))
                if masks is None or not masks.is_dir() or not body.get('masks_import'):raise ValueError('masks_import must be a folder under the data directory')
                command+=['--masks',str(masks)]
            elif body.get('masks_import'):raise ValueError('choose the import mask provider to use masks_import')
            options=body.get('photo_options',[])
            if not isinstance(options,list) or len(options)%2 or len(options)>4 or not all(isinstance(item,str) for item in options):raise ValueError('invalid capture photo options')
            seen=set()
            for flag,value in zip(options[::2],options[1::2]):
                if flag in seen:raise ValueError('duplicate capture photo option')
                seen.add(flag)
                if flag=='--object-crop-padding':
                    number=float(value)
                    if not math.isfinite(number) or not (0<=number<=1 or number==-1):raise ValueError('invalid object crop padding')
                    command+=['--object-crop-padding',value]
                elif flag=='--turntable-turn-sizes':
                    if not re.fullmatch(r'\d+(,\d+)*',value) or len(value.split(','))>16 or any(int(v)<1 for v in value.split(',')):raise ValueError('invalid capture group counts')
                    command+=['--capture-groups',value]
                else:raise ValueError('unsupported capture photo option')
            calibration=body.get('calibration')
            if isinstance(calibration,str) and calibration.startswith('fov:'):
                value=float(calibration[4:])
                if not math.isfinite(value) or not 1<value<179: raise ValueError('invalid field of view')
                command+=['--fov',str(value)]
            elif calibration and calibration != 'auto':
                path=inside(self.data,self.data/str(calibration))
                if path is None or not path.is_file(): raise ValueError('calibration must be a file under the data directory')
                command += ['--calibration',str(path)]
        else:
            command=[self.python,'-m','scripts.photo_regression.prepared_capture','--inputs',str(source),
                '--output',str(output),'--tools',str(self.capture_tools),'--base-surface',base]
            if body.get('validation_inputs'):
                validation=inside(self.data,self.data/str(body['validation_inputs']))
                if validation is None or not (validation/'cameras.json').is_file():raise ValueError('validation_inputs must be a folder under the data directory')
                command+=['--validation-inputs',str(validation)]
        if body.get('surface'): raise ValueError('this capture engine offers silhouette and Gaussian surfaces; use the native engine for the neural surface')
        splats=body.get('splats')
        if splats is not None:
            if not isinstance(splats,dict): raise ValueError('invalid Gaussian settings')
            quality=splats.get('quality','capture'); settings=splats.get('settings',[])
            if quality not in {'capture','fast','standard','best'} or not isinstance(settings,list): raise ValueError('invalid Gaussian quality')
            command += ['--gaussian','--gaussian-quality',quality]
            for item in settings:
                if not isinstance(item,str) or not re.fullmatch(r'(steps|crop|mesh|seed)=\d+',item): raise ValueError('invalid Gaussian override')
                command += ['--gaussian-set',item]
        if body.get('texture'): command+=['--texture']
        if body.get('settings'):
            if key=='photos': raise ValueError('capture preparation uses its fixed default recipe; use prepared inputs for dense overrides')
            overrides=[f"{k}={','.join(map(str,v)) if isinstance(v,list) else v}" for k,v in body['settings'].items()]
            build(None,overrides)
            for item in overrides:command+=['--set',item]
        if key=='inputs':
            record={'inputs':str(source.relative_to(self.data)),'sha256':inventory(source,['.'])}
            if body.get('validation_inputs'):record.update(validation_inputs=str(validation.relative_to(self.data)),validation_sha256=inventory(validation,['.']))
            (self.runs/(run_id+'.recovered-scene.json')).write_text(json.dumps(record)+'\n')
        with (self.runs/(run_id+'.driver.log')).open('w') as log:
            self.capture_child=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT,cwd=REPOSITORY,env={**os.environ,'PYTHONPATH':str(REPOSITORY),'RAYON_NUM_THREADS':'2'},start_new_session=True)
        deadline=time.monotonic()+10
        while time.monotonic()<deadline and not (output/'events.jsonl').exists():
            if self.capture_child.poll() is not None:break
            time.sleep(.05)
        if not (output/'events.jsonl').exists():raise RuntimeError('capture run did not start; see engine driver log')
        return {'id':run_id}

    def refine_run(self,run_id):
        if not self.capture_tools:raise ValueError('capture refinement is unavailable')
        folder=self.run_folder(run_id)
        if folder is None:raise ValueError('unknown run')
        record=folder/'recovered-scene.json'
        if not record.is_file():record=self.runs/(run_id+'.recovered-scene.json')
        if not record.is_file():raise ValueError('this run has no recovered scene to refine')
        recovered=json.loads(record.read_text())
        inputs=inside(self.data,self.data/recovered['inputs'])
        if inputs is None or not (inputs/'cameras.json').is_file():raise ValueError('recovered scene is unavailable')
        if 'sha256' in recovered and inventory(inputs,['.'])!=recovered['sha256']:raise ValueError('recovered scene changed; start from the original photos again')
        pipeline=folder/'pipeline.json'
        base=json.loads(pipeline.read_text()).get('base_surface','silhouette') if pipeline.is_file() else 'silhouette'
        if base not in {'silhouette','stereo'}:raise ValueError('invalid saved base surface')
        body={'name':run_id[-30:]+'-gaussian','inputs':str(inputs.relative_to(self.data)),
              'base_surface':base,'splats':{'quality':'capture'}}
        if recovered.get('validation_inputs'):
            validation=inside(self.data,self.data/recovered['validation_inputs'])
            if validation is None or not (validation/'cameras.json').is_file():raise ValueError('recovered scene is unavailable')
            if inventory(validation,['.'])!=recovered.get('validation_sha256'):raise ValueError('recovered scene changed; start from the original photos again')
            body['validation_inputs']=str(validation.relative_to(self.data))
        return self.start_run(body)

    def start_run(self, body):
        if not isinstance(body, dict):
            raise ValueError("JSON object required")
        if self.capture_tools:
            with self.capture_lock:
                return self.capture_run(body)
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
                return self.reply(engine.health())
            if method == "POST" and parts == ["api", "uploads"] and engine.capture_tools:
                query=parse_qs(url.query); folder=query.get('folder',[''])[0]; name=query.get('name',[''])[0]
                target=inside(engine.data,engine.data/folder/name)
                length=int(self.headers.get('Content-Length') or 0)
                if (target is None or Path(name).name!=name or not re.fullmatch(r'(capture|photos)-[A-Za-z0-9_.-]+',folder) or not name or '..' in Path(folder).parts
                        or target.suffix.lower() not in {'.jpg','.jpeg','.png','.json'} or not 0<length<=64*1024*1024):
                    return self.reply({'error':'invalid upload'},HTTPStatus.BAD_REQUEST)
                if shutil.disk_usage(engine.data).free < 8.8*2**30+length: return self.reply({'error':'free-space reserve reached'},HTTPStatus.INSUFFICIENT_STORAGE)
                if target.exists(): return self.reply({'error':'upload already exists'},HTTPStatus.CONFLICT)
                target.parent.mkdir(parents=True,exist_ok=True)
                pending=target.with_suffix(target.suffix+'.pending')
                owns_pending=False
                try:
                    remaining=length
                    with pending.open('xb') as stream:
                        owns_pending=True
                        while remaining:
                            block=self.rfile.read(min(1<<20,remaining))
                            if not block: raise ValueError('incomplete upload')
                            stream.write(block); remaining-=len(block)
                    pending.replace(target)
                except (OSError,ValueError):
                    if owns_pending:pending.unlink(missing_ok=True)
                    return self.reply({'error':'upload failed'},HTTPStatus.BAD_REQUEST)
                return self.reply({'path':str(target.relative_to(engine.data))},HTTPStatus.CREATED)
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
                if method == "POST" and parts[3] == "refine":
                    try: return self.reply(engine.refine_run(parts[2]),HTTPStatus.CREATED)
                    except (ValueError,RuntimeError) as problem:return self.reply({'error':str(problem)},HTTPStatus.BAD_REQUEST)
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
    parser.add_argument("--capture-tools",type=Path,help="Host-owned permissive capture tools configuration; enables automated real-capture pipeline")
    args = parser.parse_args()
    if args.host not in ("127.0.0.1", "localhost", "::1") and not args.token:
        parser.error("--token is required when binding to a non-local address")
    engine = Engine(args.runs, args.data, args.python, args.torch_python, args.device, args.capture_tools)
    server = ThreadingHTTPServer((args.host, args.port), handler_for(engine, args.static, args.token))
    print(f"crispmesh engine on http://{args.host}:{args.port}  runs={engine.runs}  data={engine.data}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
