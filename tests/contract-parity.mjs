// Node >=22.18: exercise the same serialized manifests through both validators.
import assert from "node:assert/strict";
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { parseProject } from "../apps/desktop/src/project.ts";

const binary = resolve(process.argv[2] ?? "build/crispmesh");
const empty = JSON.parse(readFileSync(new URL("./contracts/empty-project.json", import.meta.url), "utf8"));
const calibrated = JSON.parse(readFileSync(new URL("./contracts/calibrated-project.json", import.meta.url), "utf8"));
const poseReady = JSON.parse(readFileSync(new URL("./contracts/pose-ready-project.json", import.meta.url), "utf8"));
const cases = [["empty", empty, true], ["calibrated", calibrated, true]];
function mutation(name, mutate, valid = false) {
  const value = structuredClone(calibrated);
  mutate(value);
  cases.push([name, value, valid]);
}
mutation("version", p => p.schemaVersion = 2);
mutation("blank name", p => p.name = "  \t");
mutation("duplicate image ID", p => p.images[1].id = p.images[0].id);
mutation("invalid units", p => p.units = "cm");
mutation("negative focal length", p => p.calibration.fx = -1);
mutation("fractional dimensions", p => p.calibration.width = 1920.5);
mutation("null calibration", p => p.calibration = null);
mutation("invalid distortion", p => p.calibration.distortion = ["0"]);
mutation("stationary board", p => p.board.rotatesWithObject = false);
mutation("missing stages", p => delete p.stages);
mutation("unknown stage", p => p.stages.surprise = "pending");
mutation("unknown stage status", p => p.stages.dense = "done");
mutation("extra metadata", p => p.notes = "Preserve source dataset", true);
for (const path of ["../secret", "/absolute", "C:/drive", "images\\file.jpg", "images//a", "images/./a", "images/../a", "images/", "images/a\u0000.jpg", "images/a\n.jpg"]) {
  mutation(`unsafe path ${JSON.stringify(path)}`, p => p.images[0].path = path);
}
cases.push(["pose-ready metadata", poseReady, true]);
function poseMutation(name, mutate, valid = false) {
  const value = structuredClone(poseReady);
  mutate(value);
  cases.push([name, value, valid]);
}
poseMutation("unsupported distortion model", p => p.calibration.distortionModel = "fisheye");
poseMutation("invalid coefficient count", p => p.calibration.distortion = [0, 0, 0]);
poseMutation("four distortion coefficients", p => p.calibration.distortion = [0, 0, 0, 0], true);
poseMutation("eight distortion coefficients", p => p.calibration.distortion = Array(8).fill(0), true);
poseMutation("unsupported dictionary", p => p.board.dictionary = "DICT_6X6_250");
poseMutation("missing dictionary with markers", p => delete p.board.dictionary);
poseMutation("AprilTag with ArUco layout", p => p.board.type = "apriltag");
poseMutation("empty marker layout", p => p.board.markers = []);
poseMutation("duplicate marker IDs", p => p.board.markers.push(structuredClone(p.board.markers[0])));
poseMutation("marker ID out of range", p => p.board.markers[0].id = 50);
poseMutation("fractional marker ID", p => p.board.markers[0].id = 0.5);
poseMutation("wrong corner count", p => p.board.markers[0].cornersMm.pop());
poseMutation("nonplanar marker", p => p.board.markers[0].cornersMm[0][2] = 0.1);
poseMutation("degenerate square", p => p.board.markers[0].cornersMm[1] = [...p.board.markers[0].cornersMm[0]]);
poseMutation("reversed winding", p => p.board.markers[0].cornersMm.reverse());
poseMutation("marker size mismatch", p => p.board.markerSizeMm *= 2);
poseMutation("rotated marker", p => {
  p.board.markers[0].cornersMm = p.board.markers[0].cornersMm.map(([x, y, z]) => [-y, x, z]);
}, true);
const temp = mkdtempSync(join(tmpdir(), "crispmesh-contracts-"));
try {
  for (const [name, project, expected] of cases) {
    const serialized = JSON.stringify(project);
    const file = join(temp, "project.json");
    writeFileSync(file, serialized);
    const result = spawnSync(binary, ["validate", file], { encoding: "utf8" });
    if (result.error) throw result.error;
    const native = JSON.parse(result.stdout);
    assert.equal(result.status === 0, expected, `${name}: native result ${result.stdout}`);
    assert.equal(native.ok, expected, `${name}: native JSON status`);
    assert.equal(parseProject(serialized).issues.length === 0, expected, `${name}: web validator`);
  }
  console.log(`${cases.length} cross-validator contract cases passed`);
} finally {
  rmSync(temp, { recursive: true, force: true });
}
