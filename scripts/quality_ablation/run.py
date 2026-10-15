#!/usr/bin/env python3
"""Predeclared six-run MVE quality ablation on the existing sparse fixture."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import time
import traceback

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "scripts/mve_spike"))
from convert_scene import convert, project
try:
    from .score import score_depth, paired_grid
except ImportError:
    from score import score_depth, paired_grid

PIN = "bf2279f161ba962072ecac85224c15e82bc5f52e"
CASES = ("estimated", "subpixel", "known_pose")
SCALES = (2, 1)
RESERVE = 10 * 1024**3


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1<<20), b""):
            h.update(block)
    return h.hexdigest()


def solve3(a, b):
    m = [list(row)+[b[i]] for i,row in enumerate(a)]
    for i in range(3):
        pivot = max(range(i,3), key=lambda j: abs(m[j][i]))
        if abs(m[pivot][i]) < 1e-12:
            raise ValueError("degenerate track")
        m[i],m[pivot] = m[pivot],m[i]
        v = m[i][i]
        for k in range(i,4): m[i][k] /= v
        for j in range(3):
            if j != i:
                v = m[j][i]
                for k in range(i,4): m[j][k] -= v*m[i][k]
    return [m[i][3] for i in range(3)]


def triangulate(point, views, calibration):
    a = [[0.0]*3 for _ in range(3)]
    b = [0.0]*3
    for obs in point["observations"]:
        camera = views[obs["imageId"]]
        R,t = camera["rotation"],camera["translationMm"]
        center = [-sum(R[3*j+i]*t[j] for j in range(3)) for i in range(3)]
        rx=(obs["pixel"][0]-calibration["cx"])/calibration["fx"]
        ry=(obs["pixel"][1]-calibration["cy"])/calibration["fy"]
        ray=[R[i]*rx+R[3+i]*ry+R[6+i] for i in range(3)]
        norm=math.sqrt(sum(v*v for v in ray))
        ray=[v/norm for v in ray]
        for i in range(3):
            for j in range(3):
                q=(1 if i==j else 0)-ray[i]*ray[j]
                a[i][j]+=q
                b[i]+=q*center[j]
    xyz=solve3(a,b)
    if any(not math.isfinite(v) or abs(v)>1e6 for v in xyz):
        raise ValueError("invalid triangulation")
    return xyz


def camera_error(views, truth):
    answer = {}
    for vid,camera in views.items():
        actual = truth[vid]
        R,t=camera["rotation"],camera["translationMm"]
        center=[-sum(R[3*j+i]*t[j] for j in range(3)) for i in range(3)]
        translation_error=math.dist(center,actual["cameraCenterMm"])
        # Rotation angle from trace(R_est R_true^T).
        trace=sum(R[3*i+j]*actual["rotation"][3*i+j] for i in range(3) for j in range(3))
        angle=math.degrees(math.acos(max(-1,min(1,(trace-1)/2))))
        answer[vid]={"center_error_mm":translation_error,"rotation_error_deg":angle}
    return answer


def rewrite_scene(scene, sparse, poses, calibration):
    ids=[v["imageId"] for v in sparse["views"]]
    seeds=[triangulate(p, poses, calibration) for p in sparse["points"]]
    reprojection=[]
    for xyz,point in zip(seeds,sparse["points"]):
        for obs in point["observations"]:
            c=poses[obs["imageId"]]
            u,v=project(calibration,c["rotation"],c["translationMm"],xyz)
            reprojection.append(math.hypot(u-obs["pixel"][0],v-obs["pixel"][1]))
    if max(reprojection)>4:
        raise ValueError(f"new tracks fail reprojection: {max(reprojection):.3f}px")
    for index,vid in enumerate(ids):
        c=poses[vid]
        path=scene/"views"/f"view_{index:04d}.mve"/"meta.ini"
        fields=path.read_text().splitlines()
        for i,line in enumerate(fields):
            if line.startswith("rotation = "):
                fields[i]="rotation = "+" ".join(f"{v:.12g}" for v in c["rotation"])
            if line.startswith("translation = "):
                fields[i]="translation = "+" ".join(f"{v:.12g}" for v in c["translationMm"])
        path.write_text("\n".join(fields)+"\n")
    bundle=scene/"synth_0.out"
    lines=bundle.read_text().splitlines()
    if lines[0]!="drews 1.0" or lines[1]!=f"{len(ids)} {len(seeds)}":
        raise ValueError("unexpected bundle layout")
    for index,vid in enumerate(ids):
        c=poses[vid]
        base=2+5*index
        for row in range(3):
            lines[base+1+row]=" ".join(f"{c['rotation'][3*row+col]:.12g}" for col in range(3))
        lines[base+4]=" ".join(f"{v:.12g}" for v in c["translationMm"])
    start=2+5*len(ids)
    for i,xyz in enumerate(seeds):
        lines[start+3*i]=" ".join(f"{v:.12g}" for v in xyz)
    bundle.write_text("\n".join(lines)+"\n")
    return {"triangulated_tracks":len(seeds),"reprojection_rms_px":math.sqrt(sum(e*e for e in reprojection)/len(reprojection)),
            "reprojection_max_px":max(reprojection)}


def build_refiner(output):
    build=ROOT/"build-opencv"
    flags=(build/"CMakeFiles/crispmesh_pose_integration.dir/flags.make").read_text()
    includes=shlex.split(re.search(r"^CXX_INCLUDES = (.*)$",flags,re.M).group(1))
    link=shlex.split((build/"CMakeFiles/crispmesh_pose_integration.dir/link.txt").read_text())
    libraries=link[link.index("libcrispmesh_core.a")+1:]
    command=["/usr/bin/c++","-O2","-std=c++20",*includes,str(HERE/"refined_poses.cc"),"-o",str(output),*libraries]
    subprocess.run(command,cwd=build,check=True,timeout=120)
    return command


def run_mve(binary, scene, scale, log):
    if shutil.disk_usage(scene).free < RESERVE + 512*1024**2:
        raise RuntimeError("10 GiB free-space reserve before dense run")
    command=["/usr/bin/time","-l",str(binary),"--master-view=1",f"--scale={scale}",
             "--neighbors=2","--local-neighbors=2","--progress=simple",str(scene)]
    environment=dict(os.environ,TMPDIR=str(ROOT/".local-tools/tmp"))
    start=time.monotonic()
    with log.open("w") as stream:
        result=subprocess.run(command,stdout=stream,stderr=subprocess.PIPE,text=True,env=environment,timeout=120)
        stream.write("\n--- time -l / stderr ---\n"+result.stderr)
    if result.returncode:
        raise RuntimeError(f"dmrecon failed ({result.returncode}): {log}")
    rss=re.search(r"(\d+)\s+maximum resident set size",result.stderr)
    return {"wall_seconds":time.monotonic()-start,"peak_rss_bytes":int(rss.group(1)) if rss else None,
            "command":command,"log":str(log)}


def execute(output):
    fixture=ROOT/"build-opencv/synthetic-sparse-fixture"
    binary=ROOT/".local-tools/mve-spike"/f"mve-{PIN}"/"apps/dmrecon/dmrecon"
    project_data=json.loads((fixture/"project.json").read_text())
    sparse=json.loads((fixture/"sparse-report.json").read_text())
    truth=json.loads((fixture/"truth.json").read_text())
    if project_data["id"]!="synthetic-sparse-r02": raise ValueError("wrong fixture id")
    if project_data["calibration"]!={"width":1600,"height":1200,"fx":1500,"fy":1500,"cx":800,"cy":600,
        "distortionModel":"opencv-radtan","distortion":[0,0,0,0]}: raise ValueError("wrong calibration")
    if len(project_data["board"]["markers"])!=4: raise ValueError("wrong board")
    for marker,xy in zip(project_data["board"]["markers"],((0,0),(260,0),(0,260),(260,260))):
        x,y=xy
        if marker["cornersMm"]!=[[x,y,0],[x+40,y,0],[x+40,y+40,0],[x,y+40,0]]:
            raise ValueError("wrong marker geometry")
    files=[fixture/name for name in ("project.json","sparse-report.json","truth.json","view-1.png","view-2.png","view-3.png")]
    files += [HERE/name for name in ("run.py","score.py","refined_poses.cc")]
    files += [ROOT/"scripts/mve_spike/convert_scene.py",binary,
              ROOT/".local-tools/mve-spike"/f"mve-{PIN}"/"apps/dmrecon/dmrecon.cc",
              ROOT/".local-tools/mve-spike"/f"mve-{PIN}"/"libs/dmrecon/dmrecon.cc"]
    manifest={"fixture_id":project_data["id"],"predeclared_cases":list(CASES),"predeclared_scales":list(SCALES),
              "settings":{"master_view":1,"neighbors":2,"local_neighbors":2,"image":"undistorted","mask":"none",
                          "edge_band_mm":3,"bad_threshold_mm":5},
              "sha256":{str(p.relative_to(ROOT)):digest(p) for p in files}}
    def verify_inputs():
        for relative,expected in manifest["sha256"].items():
            if digest(ROOT/relative)!=expected:
                raise RuntimeError(f"source/input/binary changed during ablation: {relative}")
    (output/"manifest.json").write_text(json.dumps(manifest,indent=2,sort_keys=True)+"\n")
    scene_base=output/"estimated"
    convert(fixture,scene_base)
    refiner=output/"refined_poses"
    build_command=build_refiner(refiner)
    manifest["refiner_build_command"]=build_command
    manifest["sha256"][str(refiner.relative_to(ROOT))]=digest(refiner)
    (output/"manifest.json").write_text(json.dumps(manifest,indent=2,sort_keys=True)+"\n")
    images=[fixture/f"view-{i}.png" for i in range(1,4)]
    verify_inputs()
    refined=json.loads(subprocess.check_output([str(refiner),*[str(p) for p in images]],text=True,timeout=30))
    verify_inputs()
    (output/"refined-poses.json").write_text(json.dumps(refined,indent=2,sort_keys=True)+"\n")
    poses={"estimated":{v["imageId"]:v for v in sparse["views"]},
           "subpixel":{v["imageId"]:v for v in refined["poses"]},
           "known_pose":{v["imageId"]:v for v in truth["views"]}}
    calibration=project_data["calibration"]
    seed_stats={"estimated":{"triangulated_tracks":None,"source":"sparse-report.json positions"}}
    for case in CASES[1:]:
        scene=output/case
        shutil.copytree(scene_base,scene)
        seed_stats[case]=rewrite_scene(scene,sparse,poses[case],calibration)
    result={"manifest":manifest,"camera_errors_vs_truth":{case:camera_error(p,poses["known_pose"]) for case,p in poses.items()},
            "seed_stats":seed_stats,"cases":{}}
    for case in CASES:
        scene=output/case
        scores={}
        pixel_maps={}
        for scale in SCALES:
            verify_inputs()
            run=run_mve(binary,scene,scale,output/f"{case}-L{scale}.log")
            verify_inputs()
            depth=scene/"views/view_0001.mve"/f"depth-L{scale}.mvei"
            score,pixels=score_depth(depth,poses[case]["view-2"],poses["known_pose"]["view-2"],
                                     calibration,truth["objectPlanes"])
            scores[f"L{scale}"]={"run":run,"score":score,"depth_sha256":digest(depth)}
            pixel_maps[scale]=pixels
            (output/"results.json").write_text(json.dumps(result,indent=2,sort_keys=True)+"\n")
        scores["paired_L2_grid"]=paired_grid(pixel_maps[2],pixel_maps[1],400,300)
        result["cases"][case]=scores
        (output/"results.json").write_text(json.dumps(result,indent=2,sort_keys=True)+"\n")
    print(json.dumps(result,indent=2,sort_keys=True))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output",type=Path,default=ROOT/"build-opencv/quality-ablation")
    args=parser.parse_args()
    output=args.output.resolve()
    if output.exists(): parser.error(f"output must be fresh: {output}")
    if shutil.disk_usage(output.parent).free < RESERVE+2*1024**3:
        parser.error("less than 12 GiB free; preserving a 10 GiB reserve")
    output.mkdir(parents=True)
    status=output/"status.json"
    status.write_text(json.dumps({"state":"running"})+"\n")
    try:
        execute(output)
    except Exception as error:
        status.write_text(json.dumps({"state":"failed","error":str(error),"traceback":traceback.format_exc()},indent=2)+"\n")
        raise
    status.write_text(json.dumps({"state":"complete"})+"\n")


if __name__=="__main__": main()
