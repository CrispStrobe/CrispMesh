#!/usr/bin/env python3
"""One frozen, board-anchored Ceres BA candidate and unmasked MVE L1 replay."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import time
import traceback

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]
sys.path.insert(0,str(ROOT))
sys.path.insert(0,str(ROOT/"scripts/mve_spike"))
from convert_scene import convert
from scripts.quality_ablation.score import score_depth
from scripts.quality_ablation.run import camera_error

PIN="bf2279f161ba962072ecac85224c15e82bc5f52e"
RESERVE=10*1024**3
LOCAL_ENV=dict(os.environ,TMPDIR=str(ROOT/".local-tools/tmp"))


def digest(path):
    h=hashlib.sha256()
    with path.open("rb") as source:
        for part in iter(lambda:source.read(1<<20),b""):h.update(part)
    return h.hexdigest()


def build_marker_reader(path):
    build=ROOT/"build-opencv"
    flags=(build/"CMakeFiles/crispmesh_pose_integration.dir/flags.make").read_text()
    includes=shlex.split(re.search(r"^CXX_INCLUDES = (.*)$",flags,re.M).group(1))
    link=shlex.split((build/"CMakeFiles/crispmesh_pose_integration.dir/link.txt").read_text())
    libs=link[link.index("libcrispmesh_core.a")+1:]
    command=["/usr/bin/c++","-O2","-std=c++20",*includes,str(HERE/"markers.cc"),"-o",str(path),*libs]
    subprocess.run(command,cwd=build,check=True,timeout=120,env=LOCAL_ENV)
    return command


def replace_scene(scene, cameras, points):
    for i,c in enumerate(cameras):
        meta=scene/"views"/f"view_{i:04d}.mve"/"meta.ini"
        lines=meta.read_text().splitlines()
        for j,line in enumerate(lines):
            if line.startswith("rotation = "):lines[j]="rotation = "+" ".join(f"{v:.12g}" for v in c["rotation"])
            if line.startswith("translation = "):lines[j]="translation = "+" ".join(f"{v:.12g}" for v in c["translationMm"])
        meta.write_text("\n".join(lines)+"\n")
    bundle=scene/"synth_0.out"
    lines=bundle.read_text().splitlines()
    if lines[0]!="drews 1.0" or lines[1]!=f"3 {len(points)}":raise ValueError("unexpected bundle")
    for i,c in enumerate(cameras):
        base=2+5*i
        for row in range(3):
            lines[base+1+row]=" ".join(f"{c['rotation'][3*row+col]:.12g}" for col in range(3))
        lines[base+4]=" ".join(f"{v:.12g}" for v in c["translationMm"])
    for i,p in enumerate(points):
        lines[2+15+3*i]=" ".join(f"{v:.12g}" for v in p)
    bundle.write_text("\n".join(lines)+"\n")


def execute(output,baseline,optimizer):
    fixture=ROOT/"build-opencv/synthetic-sparse-fixture"
    sparse=json.loads((fixture/"sparse-report.json").read_text())
    project=json.loads((fixture/"project.json").read_text())
    if project["id"]!="synthetic-sparse-r02" or len(sparse["views"])!=3 or len(sparse["points"])!=1189:
        raise ValueError("unexpected fixture")
    if project["calibration"]!={"width":1600,"height":1200,"fx":1500,"fy":1500,"cx":800,"cy":600,
                                   "distortionModel":"opencv-radtan","distortion":[0,0,0,0]}:
        raise ValueError("unexpected calibration")
    ids=[v["imageId"] for v in sparse["views"]]
    if ids!=["view-1","view-2","view-3"]:raise ValueError("unexpected view order")
    if [p["id"] for p in sparse["points"]]!=list(range(1189)):
        raise ValueError("unexpected track ids")
    binary=ROOT/".local-tools/mve-spike"/f"mve-{PIN}"/"apps/dmrecon/dmrecon"
    files=[fixture/name for name in ("project.json","sparse-report.json","view-1.png","view-2.png","view-3.png")]
    files += [HERE/name for name in ("run.py","markers.cc","optimize.cc","CMakeLists.txt")]
    files += [ROOT/"scripts/mve_spike/convert_scene.py",ROOT/"scripts/quality_ablation/score.py",
              ROOT/".local-tools/bundle-quality/ceres-2.2.0.tar.gz",
              ROOT/".local-tools/bundle-quality/eigen-3.4.0.tar.gz",binary,optimizer]
    manifest={"candidate":"single frozen Huber-1px joint BA", "max_iterations":30,"marker_weight":1,
              "track_weight":1,"intrinsics":"fixed","marker_world_points":"fixed board millimetres",
              "mve":{"master_view":1,"scale":1,"neighbors":2,"local_neighbors":2,"mask":"none"},
              "sha256":{str(p.relative_to(ROOT)):digest(p) for p in files}}
    baseline_file=baseline/"results.json"
    manifest["reference_baseline_results_sha256"]=digest(baseline_file)
    def verify():
        for name,expected in manifest["sha256"].items():
            if digest(ROOT/name)!=expected:raise RuntimeError(f"input changed: {name}")
    marker_reader=output/"read_markers"
    manifest["marker_build_command"]=build_marker_reader(marker_reader)
    manifest["sha256"][str(marker_reader.relative_to(ROOT))]=digest(marker_reader)
    (output/"manifest.json").write_text(json.dumps(manifest,indent=2,sort_keys=True)+"\n")
    selftest=subprocess.run([str(optimizer),"--self-test"],capture_output=True,text=True,timeout=30,env=LOCAL_ENV)
    (output/"self-test.log").write_text(selftest.stdout+selftest.stderr)
    if selftest.returncode:raise RuntimeError("optimizer self-test failed")
    verify()
    image_paths=[fixture/f"view-{i}.png" for i in range(1,4)]
    markers=subprocess.check_output([str(marker_reader),*[str(p) for p in image_paths]],text=True,timeout=30,env=LOCAL_ENV).splitlines()
    if len(markers)!=48:raise RuntimeError("expected 48 default-detector board corners")
    counts=[sum(int(line.split()[0])==i for line in markers) for i in range(3)]
    if counts!=[16]*3:raise RuntimeError(f"unequal marker observations: {counts}")
    observations=[(i,p["id"],o["pixel"][0],o["pixel"][1])
                  for p in sparse["points"] for o in p["observations"]
                  for i in [ids.index(o["imageId"])]]
    if len(observations)!=2843:raise RuntimeError("track observation count changed")
    lines=["C 3"]
    for view in sparse["views"]:
        lines.append(" ".join(str(v) for v in view["rotation"]+view["translationMm"]))
    lines.append("P 1189")
    lines.extend(" ".join(str(v) for v in p["positionMm"]) for p in sparse["points"])
    lines.append("O 2843")
    lines.extend(f"{i} {pid} {u} {v}" for i,pid,u,v in observations)
    lines.append("M 48")
    lines.extend(" ".join(line.split()[0:1]+line.split()[3:]) for line in markers)
    data=output/"optimization-input.txt"
    data.write_text("\n".join(lines)+"\n")
    manifest["sha256"][str(data.relative_to(ROOT))]=digest(data)
    (output/"manifest.json").write_text(json.dumps(manifest,indent=2,sort_keys=True)+"\n")
    optimized=output/"optimized.json"
    command=["/usr/bin/time","-l",str(optimizer),str(data),str(optimized)]
    start=time.monotonic()
    with (output/"optimizer.log").open("w") as stream:
        run=subprocess.run(command,stdout=stream,stderr=subprocess.PIPE,text=True,timeout=120,env=LOCAL_ENV)
        stream.write("\n--- time -l / stderr ---\n"+run.stderr)
    runtime=time.monotonic()-start
    manifest["optimizer_run"]={"command":command,"wall_seconds":runtime,"exit_code":run.returncode,
                                "peak_rss_bytes":int(match.group(1)) if (match:=re.search(r"(\d+)\s+maximum resident set size",run.stderr)) else None}
    (output/"manifest.json").write_text(json.dumps(manifest,indent=2,sort_keys=True)+"\n")
    verify()
    if run.returncode:raise RuntimeError(f"fixed BA failed with status {run.returncode}; see optimized.json and optimizer.log")
    result=json.loads(optimized.read_text())
    if not result["usable"] or result["final"]["nonpositive_depths"] or len(result["points"])!=1189:
        raise RuntimeError("optimized solution failed declared constraints")
    scene=output/"scene"
    convert(fixture,scene)
    replace_scene(scene,result["cameras"],result["points"])
    if shutil.disk_usage(output).free < RESERVE+512*1024**2:
        raise RuntimeError("10 GiB free-space reserve")
    command=["/usr/bin/time","-l",str(binary),"--master-view=1","--scale=1","--neighbors=2",
             "--local-neighbors=2","--progress=simple",str(scene)]
    verify()
    start=time.monotonic()
    with (output/"mve.log").open("w") as stream:
        dense=subprocess.run(command,stdout=stream,stderr=subprocess.PIPE,text=True,timeout=120,env=LOCAL_ENV)
        stream.write("\n--- time -l / stderr ---\n"+dense.stderr)
    manifest["mve_run"]={"command":command,"wall_seconds":time.monotonic()-start,"exit_code":dense.returncode,
                         "peak_rss_bytes":int(match.group(1)) if (match:=re.search(r"(\d+)\s+maximum resident set size",dense.stderr)) else None}
    (output/"manifest.json").write_text(json.dumps(manifest,indent=2,sort_keys=True)+"\n")
    verify()
    if dense.returncode:raise RuntimeError("MVE L1 failed")
    # Truth is opened only after the optimization and dense reconstruction.
    truth_path=fixture/"truth.json"
    truth=json.loads(truth_path.read_text())
    manifest["sha256"][str(truth_path.relative_to(ROOT))]=digest(truth_path)
    depth=scene/"views/view_0001.mve/depth-L1.mvei"
    score,_=score_depth(depth,result["cameras"][1],truth["views"][1],project["calibration"],truth["objectPlanes"])
    if digest(baseline_file)!=manifest["reference_baseline_results_sha256"]:
        raise RuntimeError("baseline results changed")
    reference=json.loads(baseline_file.read_text())["cases"]["estimated"]["L1"]["score"]
    report={"optimizer":{key:result[key] for key in ("initial_cost","final_cost","iterations","termination","usable","initial","final")},
            "camera_error_vs_truth":camera_error({ids[i]:dict(result["cameras"][i]) for i in range(3)},
                                                   {v["imageId"]:v for v in truth["views"]}),
            "depth_score":score,"original_L1_score":reference,"depth_sha256":digest(depth)}
    (output/"manifest.json").write_text(json.dumps(manifest,indent=2,sort_keys=True)+"\n")
    (output/"results.json").write_text(json.dumps(report,indent=2,sort_keys=True)+"\n")
    print(json.dumps(report,indent=2,sort_keys=True))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output",type=Path,default=ROOT/"build-opencv/bundle-quality")
    p.add_argument("--baseline",type=Path,default=ROOT/"build-opencv/quality-ablation-final")
    p.add_argument("--optimizer",type=Path,
                   default=ROOT/".local-tools/bundle-quality/quality-build-mpl/bundle_quality_optimize")
    args=p.parse_args();output=args.output.resolve()
    if output.exists():p.error("output must be fresh")
    if shutil.disk_usage(output.parent).free<RESERVE+2*1024**3:p.error("10 GiB reserve plus 2 GiB headroom")
    output.mkdir(parents=True)
    status=output/"status.json"
    status.write_text('{"state":"running"}\n')
    try:execute(output,args.baseline.resolve(),args.optimizer.resolve())
    except Exception as error:
        status.write_text(json.dumps({"state":"failed","error":str(error),"traceback":traceback.format_exc()},indent=2)+"\n")
        raise
    status.write_text('{"state":"complete"}\n')


if __name__=="__main__":main()
