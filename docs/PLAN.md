# CrispMesh implementation plan

## Active roadmap (2026-09-27)

The approved [quality-first roadmap](SOTA-ROADMAP.md) supersedes the historical
board-first sequence and permissive/MPL-only desktop policy below. Ordinary
rigid-object photos with unknown poses are required; markers are optional. Original
project code is now AGPL-3.0-only. Third-party and App Store clearance remain separate.
The current parallel batch is S02 classical CPU backend, S03 object-motion camera
handling and S04 surface evaluation, with root review and live comparison gates.
The following foundation plan remains historical context, not a claim that its
later reconstruction/product tasks have shipped.

## Product and decisions

Build a calibrated LEGO turntable scanner: import still photographs, verify calibration and rotating-board poses, reconstruct an object, inspect scale/coverage, and export a mesh. macOS is the first full reconstruction target; Windows and Linux share the desktop architecture. Mobile initially captures/transfers datasets; the browser initially inspects projects. Neither implies local dense reconstruction.

Use Tauri 2 + TypeScript for the desktop application and a portable C++20 library with a small versioned C ABI and CLI for computation. A desktop worker process will isolate heavy work; mobile will call the library directly. Keep capture hardware, UI, and platform APIs outside the core. Use a replaceable dense backend. Start with OpenCV for calibrated geometry, evaluate OpenMVG/Ceres/Eigen for sparse optimization, selected MVE for density, PoissonRecon for surfaces, and meshoptimizer for later optimization. Do not import this whole stack before the evaluation gates pass.

Policy: allow permissive licenses and MPL-2.0; reject GPL/AGPL in shipped components; LGPL and unknown/custom terms need explicit review. Exact sources, configuration, transitive dependencies and release artifacts determine approval. System runtimes are recorded separately. Do not imply App Store approval or legal clearance.

## Delivery sequence and scoped tasks

| ID | Scope | Dependencies | Acceptance / gate |
| --- | --- | --- | --- |
| F01 | Versioned project and event contracts, coordinate conventions, errors, stages | None | Documented inputs/outputs, millimetres, object-to-camera transform, relative paths, capability discovery |
| F02 | Dependency-free C++ core/C ABI/CLI foundation | F01 | Build and meaningful tests on macOS; capabilities, project validation, synthetic geometry diagnostic; unavailable reconstruction explicitly fails |
| F03 | Tauri desktop shell and browser-compatible inspection workspace | F01 | Typecheck/build; import/validate project JSON; real project state; clear backend capability status, no simulated scan result |
| F04 | License policy, pinned dependency inventory/check and CI | None | Unknown/rejected entries fail; candidate engines are not mislabeled approved; native/web checks automated |
| R01 | Minimal pinned OpenCV build + calibration/marker pose | F01,F02,F04 | Synthetic known poses and real calibrated board images; pose residuals and scale reported; no stationary-board assumption |
| R02 | Masked object feature tracks and triangulation | R01 | Reject background/board pixels, low parallax and negative depths; measured reprojection residuals |
| R03 | Dense-backend comparison and integration spike | R02,F04 | Compare calibrated OpenCV stereo baseline and dedicated multi-view candidates; prove mask propagation and pose conversion, measure depth/normal quality. MVE additionally requires valid sparse seeds. Reject candidates that fail quality or license gates |
| R04 | Oriented cloud → Poisson mesh → export | R03 | Valid normals, units and bounded memory; measured dimensions; real scan CLI smoke test |
| R05 | Jobs, progress, cancellation, atomic artifacts and resume | F02,R01 | Stable JSONL events, process cancellation, content/settings fingerprints, interrupted-run recovery; no reuse of stale artifacts |
| D01 | Desktop worker integration and project persistence | F03,R05 | Bundle worker per target, scoped native commands, cancellation; roundtrip projects with relative paths |
| D02 | Three.js cloud/mesh/camera inspection + export | D01,R04 | Display real outputs, dimension/coverage diagnostics; no generated sample presented as reconstructed data |
| Q01 | Measured regression scans and resource budgets | R01 onward | Textured object, glossy LEGO, low texture, thin geometry; track dimensions, residuals, completeness, runtime/RSS and repeatability |
| P01 | Windows/Linux validation and signed desktop packages | D02,Q01,F04 | Native builds and scan regression per OS, complete notices/source bundles; platform runtime inventory |
| P02 | Mobile capture / LEGO move-settle-capture-ack protocol | F01,Q01 | Actual hardware transport chosen and tested; persist settings and actual capture status; device transfer roundtrip |
| P03 | Native mobile core feasibility; browser worker/WASM spike | P02,R04 | Memory/thermal benchmarks, cancellation, bounded workload; separate go/no-go per platform |

Current execution scope: F01–F04 foundation delivered; R01 calibrated image pose estimation and R02 masked sparse tracks implemented and verified on rendered images. D02 includes manual pose/sparse inspection and an interactive point cloud. R03a test-only stereo evaluation has run against a checksummed real measured-reference scene; [R03b–e](DENSE-EVALUATION.md) now scope rectification, reconstruction masks, backend comparison and oriented fusion. Real-turntable validation remains open. Reconstruction is not complete until one measured real dataset passes the full CLI pipeline. Do not mark later milestones complete through mocks or placeholder success responses.

User clarification: the originally discussed libraries were suggestions, not constraints. MVE is one dense-backend candidate, not an architectural dependency. Select or replace libraries based on measured quality, masking, maintainability, platform fit, integration cost and the license policy.

Existing applications are also reuse candidates. [The 3DLiveScanner review](REPOSITORY-REUSE.md) identifies mobile capture, recorded-session handling and texture projection as useful references, but its AR SDK/Tango binary reconstruction is not a drop-in desktop photo-MVS backend. Review existing implementations before adding new algorithms; integrate only scoped, audited components.

Testing clarification: separate GPL development oracles are permitted; shipped-component restrictions remain unchanged. R03 now starts with a test-only rectified stereo evaluator, checksummed measured benchmark data, invariant/unit tests and a reproducible live browser check. These are scoped evaluation tasks, not an announcement of a production dense engine. See [TESTING.md](TESTING.md) for the distinction between measured truth, regression goldens and oracle comparisons.

## Sequencing and task boundaries

The active quality/performance workstream is scoped in [QUALITY-IMPROVEMENT.md](QUALITY-IMPROVEMENT.md): diagnostic error decomposition, fixed stereo profiles and timings, calibrated rectification, conservative multi-view consistency/fusion, then image-derived and measured-object acceptance. Improving benchmark coverage alone is not the completion criterion.

The critical path is F01/F02 → R01 → R02 → R03 → R04 → measured scan acceptance. F03 and F04 run in parallel with the core foundation; R05 can run alongside the geometry stages after their artifact contracts stabilize. D01/D02 depend on working core stages. Native mobile reconstruction and browser WASM remain separate feasibility projects.

R01's board summary now includes dictionary identifiers, marker IDs, measured corner coordinates and a calibration distortion model; see POSE-MILESTONE.md. The implemented detector-to-pose path is tested on rendered images; its second gate is measured real-image validation. R02's observations, binary masks, bounded matching and triangulation rejection thresholds are defined in SPARSE-MILESTONE.md and exercised through the production CLI. Before R03, document explicit pixel/focal/pose conversion into each candidate and prove that masked pixels cannot seed or contribute to the reconstructed object. Each backend experiment must emit data that can be independently inspected, not just a success code.

Keep each implementation task to one independently reviewable artifact or behavior; split R01–R05 further before coding. Treat R03 as high uncertainty with a go/no-go report, not a fixed delivery estimate. If MVE fails the mask/quality gate, evaluate a different dense backend or scope a custom matcher as a new algorithm project before committing to a UI delivery date.

Foundation status and exact verification are recorded in `docs/STATUS.md`. Unimplemented milestones remain open even when exploratory source inspection or compilation has succeeded.

## Project / job contract v1

Canonical project JSON fields: `schemaVersion: 1`, `id` (nonempty string), `name` (nonempty string), `units: "mm"`, `images` (array of `{id,path,maskPath?}`), optional `calibration` (`width,height,fx,fy,cx,cy,distortion`), optional `board` (`type:"aruco"|"apriltag",markerSizeMm,rotatesWithObject:true`), and `stages` (map from stage name to `pending|running|complete|failed|unavailable`). Image and mask paths are relative to the project root without traversal. Stages: calibration, poses, sparse, dense, mesh, export. Image IDs are unique. An empty image list is a valid new project, not a reconstruction-ready scan.

Coordinate convention: right-handed object coordinates in millimetres; camera x right, y down, z forward. Pose is object-to-camera `Xc = R Xo + t`; document row-major storage before adding matrices. Calibration belongs to the image dimensions; resized inputs require scaled intrinsics. Original files are immutable.

Future worker protocol: one JSON object per stdout line, stderr for logs; protocolVersion, jobId, sequence, type, stage, progress and structured error. Terminal states are succeeded/failed/cancelled. Only succeeded artifacts can be resumed, subject to source/settings/backend fingerprints. Protocol implementation belongs to R05, not a fabricated foundation job runner.

## Work assignment and review

Sol core agent owns `core/`, root `CMakeLists.txt`, and native tests; Sol app agent owns `apps/desktop/`; Sol dependency agent owns `dependencies/`, `scripts/`, `.github/`, and dependency/backend evaluation notes. Main agent owns this plan, shared integration, README and final verification. Agents must preserve each other's files, expose limitations, and report exact tests. No implicit hardware transport, remote service, or download-and-execute engine path.

## Upstream references checked 2026-09-26

- Tauri architecture: https://v2.tauri.app/concept/architecture/
- Sidecar packaging and per-target naming: https://v2.tauri.app/develop/sidecar/
- OpenMVG source and license: https://github.com/openMVG/openMVG
- OpenMVG third-party inventory: https://openmvg.readthedocs.io/en/latest/third_party/third_party/
- MVE source and license (must be audited at selected commit): https://github.com/simonfuhrmann/mve

These are architecture references, not a completed dependency or legal audit. The foundation dependency report must distinguish tooling, shipped components, and unintegrated candidates.
