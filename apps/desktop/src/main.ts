import { invoke } from "@tauri-apps/api/core";
import { generateBoardGrid, newProject, parseProject, readiness, STAGES, validateProject, validRelativePath, type Board, type Calibration, type Project, type ValidationIssue } from "./project";
import { parsePoseReport, type PoseReport } from "./poseReport";
import { parseSparseReport, type SparseReport } from "./sparseReport";
import { mountSparseViewer } from "./sparseViewer";
import "./style.css";
import "./recent.css";
import "./setup.css";
import "./poseReport.css";
import "./sparseReport.css";

const STORAGE_KEY = "crispmesh.project.v1";
const LIBRARY_KEY = "crispmesh.library.v1";
const app = document.querySelector<HTMLDivElement>("#app")!;
let project: Project | null = null;
let library: Record<string, Project> = Object.create(null);
let importIssues: ValidationIssue[] = [];
let notice = "";
let nativeStatus = "Browser workspace · reconstruction unavailable";
let poseReport: PoseReport | null = null;
let poseReportName = "";
let poseReportIssues: ValidationIssue[] = [];
let sparseReport: SparseReport | null = null;
let sparseReportName = "";
let sparseReportIssues: ValidationIssue[] = [];
let inputRevision = 0;
let sparseImportRevision = 0;
let disposeSparseViewer: (() => void) | null = null;

const escapeHtml = (value: unknown) => String(value ?? "").replace(/[&<>"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]!);
const slug = (value: string) => value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "scan";

function restore() {
  let saved: string | null;
  try {
    const rawLibrary = localStorage.getItem(LIBRARY_KEY);
    if (rawLibrary) {
      const candidate: unknown = JSON.parse(rawLibrary);
      if (candidate && typeof candidate === "object" && !Array.isArray(candidate)) {
        library = Object.assign(Object.create(null), Object.fromEntries(Object.entries(candidate).filter(([, value]) => !validateProject(value).length))) as Record<string, Project>;
      }
    }
    saved = localStorage.getItem(STORAGE_KEY);
  } catch { notice = "Browser storage is unavailable. Download your project JSON before closing this page."; return; }
  if (!saved) return;
  const parsed = parseProject(saved);
  if (parsed.project) project = parsed.project;
  else notice = "Saved project could not be restored; import a valid project or start a new one.";
}

function persist() {
  if (!project) return;
  try {
    const serialized = JSON.stringify(project);
    localStorage.setItem(STORAGE_KEY, serialized);
    if (!validateProject(project).length) {
      library[project.id] = structuredClone(project);
      localStorage.setItem(LIBRARY_KEY, JSON.stringify(library));
    }
  } catch { notice = "Browser storage is unavailable or full. Download your project JSON to keep these changes."; }
}

function issuesHtml(issues: ValidationIssue[], title = "Project needs attention") {
  return issues.length ? `<div class="issues" role="alert"><strong>${escapeHtml(title)}</strong><ul>${issues.map((issue) => `<li><code>${escapeHtml(issue.path)}</code> ${escapeHtml(issue.message)}</li>`).join("")}</ul></div>` : "";
}

function render() {
  disposeSparseViewer?.();
  disposeSparseViewer = null;
  const issues = project ? validateProject(project) : [];
  const ready = project && !issues.length ? readiness(project) : null;
  app.innerHTML = `
    <header class="topbar"><div class="brand"><span class="brand-mark">C<span>3</span></span><div><strong>CrispMesh</strong><small>SCAN WORKSPACE</small></div></div><span class="top-status"><span class="pulse"></span>${escapeHtml(nativeStatus)}</span></header>
    <div class="layout"><aside class="sidebar"><div class="sidebar-title">WORKSPACE</div><a class="nav-link active" href="#project"><span>◈</span> Project overview</a><a class="nav-link" href="#images"><span>▦</span> Source images</a><a class="nav-link" href="#setup"><span>◎</span> Capture setup</a><a class="nav-link" href="#pipeline"><span>⌁</span> Pipeline</a><div class="sidebar-title recent-title">SAVED PROJECTS</div><div class="recent-list">${Object.values(library).reverse().map((item) => `<button class="recent-project ${project?.id === item.id ? "selected" : ""}" type="button" data-id="${escapeHtml(item.id)}">${escapeHtml(item.name)}</button>`).join("") || `<span class="recent-empty">No saved projects</span>`}</div><div class="sidebar-foot"><span class="small-dot"></span> Foundation build <span class="sidebar-version">v0.1</span></div></aside>
    <main class="content"><div class="page-heading"><div><span class="eyebrow">CALIBRATED TURNTABLE SCANNING</span><h1>${project ? escapeHtml(project.name) : "Your scan workspace"}</h1><p>${project ? "Inspect and prepare a project for calibrated reconstruction." : "Create a project or inspect an existing scan manifest."}</p></div><div class="heading-actions"><button id="import-button" class="button subtle" type="button">Import JSON</button><button id="new-button" class="button primary" type="button">New project</button></div></div>
      ${notice ? `<div class="notice" role="status">${escapeHtml(notice)}</div>` : ""}
      ${issuesHtml(importIssues)}
      ${!project ? `<section class="empty-state"><div class="empty-icon">◈</div><h2>Start with the scan record</h2><p>A CrispMesh project keeps image paths, calibration, marker-board details, and stage metadata in one portable JSON file.</p><form id="create-form" class="create-form"><label for="new-name">Project name</label><div><input id="new-name" name="name" placeholder="e.g. Red LEGO car" required /><button class="button primary" type="submit">Create project</button></div></form><p class="quiet">Projects are saved in this browser until you download their JSON. Images remain in your own project folder.</p></section>` : `
        <section id="project" class="section"><div class="section-head"><div><span class="eyebrow">PROJECT</span><h2>Overview</h2></div><button id="download-button" class="button dark" type="button" ${issues.length ? "disabled" : ""}>Download project JSON</button></div>
        <div class="overview-grid"><div class="card overview-main"><div class="card-top"><span class="round-icon">◈</span><span class="badge ${issues.length ? "bad" : ready?.label === "Inputs recorded" ? "good" : "warn"}">${issues.length ? "Invalid project" : escapeHtml(ready?.label)}</span></div><label class="field"><span>Project name</span><input id="project-name" value="${escapeHtml(project.name)}" /></label><div class="meta-line"><span>Project ID</span><code>${escapeHtml(project.id)}</code></div><div class="meta-line"><span>Schema</span><span>v${project.schemaVersion} · ${escapeHtml(project.units)}</span></div></div><div class="card readiness"><span class="eyebrow">READINESS</span><h3>${issues.length ? "Fix the project record" : escapeHtml(ready?.label)}</h3><ul>${(issues.length ? issues.map((issue) => `${issue.path}: ${issue.message}`) : ready?.details ?? []).map((detail) => `<li>${escapeHtml(detail)}</li>`).join("")}</ul><p class="quiet">Readiness only checks the project record. It does not inspect image files or run a scan.</p></div></div></section>
        <section id="images" class="section"><div class="section-head"><div><span class="eyebrow">SOURCE DATA</span><h2>Images <span class="count">${project.images.length}</span></h2></div></div><div class="card table-card">${project.images.length ? `<div class="image-table"><div class="table-header"><span>IMAGE ID</span><span>RELATIVE IMAGE PATH</span><span>MASK PATH</span><span></span></div>${project.images.map((image, index) => `<div class="image-row"><code>${escapeHtml(image.id)}</code><span title="${escapeHtml(image.path)}">${escapeHtml(image.path)}</span><span title="${escapeHtml(image.maskPath ?? "")}">${image.maskPath ? escapeHtml(image.maskPath) : `<em>Not supplied</em>`}</span><button class="icon-button remove-image" data-index="${index}" type="button" aria-label="Remove image ${escapeHtml(image.id)}">×</button></div>`).join("")}</div>` : `<div class="table-empty">No images recorded yet. Add paths relative to the project folder.</div>`}<form id="add-image-form" class="add-image"><input name="id" placeholder="Image ID" aria-label="Image ID" required /><input name="path" placeholder="images/frame-001.jpg" aria-label="Relative image path" required /><input name="maskPath" placeholder="masks/frame-001.png (optional)" aria-label="Relative mask path" /><button class="button subtle" type="submit">Add image</button></form></div><p class="help">This manifest stores paths only. Place the photos and masks beside the downloaded project file before processing them with the CLI.</p></section>
        <section id="setup" class="section"><div class="section-head"><div><span class="eyebrow">CAPTURE GEOMETRY</span><h2>Calibration &amp; rotating board</h2></div></div><div class="setup-grid"><div class="card setup-card"><div class="setup-heading"><span class="round-icon blue">◎</span><div><h3>Camera intrinsics</h3><p>For the original image dimensions</p></div></div><form id="calibration-form"><div class="field-grid">${(["width","height","fx","fy","cx","cy"] as const).map((key) => `<label class="field"><span>${key}</span><input type="number" step="any" name="${key}" value="${escapeHtml(project!.calibration?.[key] ?? "")}" placeholder="${key}" required /></label>`).join("")}</div><label class="field"><span>Distortion coefficients, comma separated</span><input name="distortion" value="${escapeHtml(project.calibration?.distortion.join(", ") ?? "")}" placeholder="0, 0, 0, 0" /></label><div class="form-actions"><button class="button subtle" type="submit">Save calibration</button>${project.calibration ? `<button class="text-button" id="remove-calibration" type="button">Remove</button>` : ""}</div></form></div><div class="card setup-card"><div class="setup-heading"><span class="round-icon amber">▣</span><div><h3>Marker board</h3><p>Fixed to the turntable</p></div></div><form id="board-form"><label class="field"><span>Marker family</span><select name="type"><option value="aruco" ${project.board?.type === "aruco" ? "selected" : ""}>ArUco</option><option value="apriltag" ${project.board?.type === "apriltag" ? "selected" : ""}>AprilTag</option></select></label><label class="field"><span>Marker side length (mm)</span><input type="number" min="0" step="any" name="markerSizeMm" value="${escapeHtml(project.board?.markerSizeMm ?? "")}" placeholder="e.g. 20" required /></label><p class="setup-note">The board must rotate rigidly with the object and stay visible in each exposure. The board layout and image observations are still needed for pose estimation.</p><div class="form-actions"><button class="button subtle" type="submit">Save board</button>${project.board ? `<button class="text-button" id="remove-board" type="button">Remove</button>` : ""}</div></form></div></div></section>
        <section id="pipeline" class="section"><div class="section-head"><div><span class="eyebrow">RECONSTRUCTION</span><h2>Pipeline</h2></div></div><div class="card pipeline-card"><div class="pipeline-intro"><span class="round-icon dark-icon">⌁</span><div><h3>Native reconstruction is not connected</h3><p>This workspace can prepare and inspect projects. A future worker will run calibration, pose estimation, dense reconstruction and export.</p></div></div><div class="stage-list">${STAGES.map((stage, index) => `<div class="stage"><span class="stage-number">${String(index + 1).padStart(2, "0")}</span><span class="stage-name">${stage[0].toUpperCase() + stage.slice(1)}</span><span class="stage-state">${escapeHtml(project!.stages[stage] ?? "not recorded")} <small>in project file · unverified</small></span></div>`).join("")}</div></div></section>
      `}
      <footer>Project format v1 · object coordinates in mm · camera x right, y down, z forward</footer>
    </main></div><input id="file-input" type="file" accept=".json,application/json" hidden />`;
  if (project) { enhanceSetup(project); enhancePoseReport(); enhanceSparseReport(); }
  if (ready?.label === "Pose inputs recorded") document.querySelector(".badge")?.classList.replace("warn", "good");
  bind();
}

function clearPoseReport() {
  inputRevision++;
  poseReport = null;
  poseReportName = "";
  poseReportIssues = [];
  sparseImportRevision++;
  sparseReport = null;
  sparseReportName = "";
  sparseReportIssues = [];
}

function enhanceSparseReport() {
  document.querySelector("#pipeline .section-head")!.insertAdjacentHTML("beforeend", `<button id="import-sparse-report" class="button subtle" type="button">Import sparse report</button>`);
  let body = `<p class="help">Use <code>crispmesh reconstruct-sparse project.json</code>, save its successful JSON output, then import it here to inspect actual sparse tracks. This app does not run reconstruction or change project stages.</p>`;
  if (sparseReport) {
    const report = sparseReport;
    const rms = Math.sqrt(report.points.reduce((sum, point) => sum + point.rmsReprojectionErrorPx ** 2, 0) / report.points.length);
    const max = report.points.reduce((largest, point) => Math.max(largest, point.maxReprojectionErrorPx), 0);
    const coverage = new Map(report.views.map((view) => [view.imageId, 0]));
    const bounds = [[Infinity, -Infinity], [Infinity, -Infinity], [Infinity, -Infinity]];
    for (const point of report.points) {
      for (const observation of point.observations) coverage.set(observation.imageId, coverage.get(observation.imageId)! + 1);
      point.positionMm.forEach((coordinate, axis) => { bounds[axis][0] = Math.min(bounds[axis][0], coordinate); bounds[axis][1] = Math.max(bounds[axis][1], coordinate); });
    }
    body = `<div class="pose-report-summary"><span class="badge good">Report loaded</span><span>${escapeHtml(sparseReportName)}</span><span>${report.points.length.toLocaleString()} points / tracks</span><span>${report.views.length} views</span></div>
      <p class="help">Imported for inspection only. The desktop app has not run or independently verified reconstruction; project stages are unchanged. Points are sparse features in the board frame, not a surface or mesh.</p>
      <div class="sparse-metrics"><div><small>TRACKS</small><strong>${report.statistics.acceptedTracks.toLocaleString()}</strong></div><div><small>CANDIDATE MATCHES</small><strong>${report.statistics.candidateMatches.toLocaleString()}</strong></div><div><small>REJECTED MATCHES</small><strong>${report.statistics.rejectedMatches.toLocaleString()}</strong></div><div><small>TRACK RMS / MAX RESIDUAL</small><strong>${escapeHtml(rms.toFixed(2))} / ${escapeHtml(max.toFixed(2))} px</strong></div></div>
      <canvas id="sparse-canvas" class="sparse-canvas" role="img" aria-label="Interactive projected three-dimensional sparse point cloud in millimetres"></canvas>
      <p class="help">Board frame bounds (mm): X ${bounds[0].map((n) => escapeHtml(n.toFixed(1))).join(" to ")} · Y ${bounds[1].map((n) => escapeHtml(n.toFixed(1))).join(" to ")} · Z ${bounds[2].map((n) => escapeHtml(n.toFixed(1))).join(" to ")}. Viewer shows up to 20,000 sampled points; counts use all points.</p>
      <div class="sparse-view-table"><div class="sparse-view-head"><span>VIEW</span><span>FEATURES</span><span>TRACK COVERAGE</span></div>${report.views.map((view) => `<div class="sparse-view-row"><span><code>${escapeHtml(view.imageId)}</code><small>${escapeHtml(view.path)}<br>Mask: ${escapeHtml(view.maskPath)}</small></span><span>${view.featureCount.toLocaleString()}</span><span>${coverage.get(view.imageId)!.toLocaleString()} tracks</span></div>`).join("")}</div>
      <p class="help">${escapeHtml(report.backend.name)} ${escapeHtml(report.backend.version)} · ${escapeHtml(report.backend.feature)} · row-major object-to-camera view rotations</p>`;
  }
  document.querySelector("#pipeline")!.insertAdjacentHTML("beforeend", `<div class="card sparse-report-card"><h3>Sparse report inspection</h3>${issuesHtml(sparseReportIssues, "Could not import sparse report")}${body}</div><input id="sparse-report-file" type="file" accept=".json,application/json" hidden />`);
  if (sparseReport) disposeSparseViewer = mountSparseViewer(document.querySelector<HTMLCanvasElement>("#sparse-canvas")!, sparseReport.points);
}

function enhancePoseReport() {
  document.querySelector("#pipeline .section-head")!.insertAdjacentHTML("beforeend", `<button id="import-pose-report" class="button subtle" type="button">Import pose report</button>`);
  const body = poseReport ? `<div class="pose-report-summary"><span class="badge good">CLI report loaded</span><span>${escapeHtml(poseReportName)}</span><span>${poseReport.poses.length} image poses</span></div><p class="help">Imported report for inspection only. The desktop app has not run or independently verified these poses; project stage status is unchanged.</p><div class="pose-table"><div class="pose-header"><span>IMAGE</span><span>MARKERS</span><span>TRANSLATION (MM)</span><span>RMS / MAX (PX)</span></div>${poseReport.poses.map((pose) => `<div class="pose-row"><div><code>${escapeHtml(pose.imageId)}</code><small>${escapeHtml(pose.path)}</small></div><span>${escapeHtml(pose.detectedMarkerIds.join(", "))}<small>${pose.cornerCount} corners${pose.ignoredMarkerIds.length ? ` · ignored ${escapeHtml(pose.ignoredMarkerIds.join(", "))}` : ""}</small></span><span>${pose.translationMm.map((n) => escapeHtml(n.toFixed(2))).join(", ")}</span><span>${escapeHtml(pose.rmsReprojectionErrorPx.toFixed(2))} / ${escapeHtml(pose.maxReprojectionErrorPx.toFixed(2))}<details><summary>Rotation matrix</summary><code>${pose.rotation.map((n, i) => `${escapeHtml(n.toFixed(6))}${i % 3 === 2 ? "<br>" : " &nbsp; "}`).join("")}</code></details></span></div>`).join("")}</div><p class="help">${escapeHtml(poseReport.backend.name)} ${escapeHtml(poseReport.backend.version)} · ${escapeHtml(poseReport.backend.dictionary)} · ${escapeHtml(poseReport.backend.solver)} · object-to-camera, row-major rotation</p>` : `<p class="help">Use <code>crispmesh estimate-poses project.json</code>, save its successful JSON output, then import it here to inspect per-image residuals. This does not run the CLI or change project stages.</p>`;
  document.querySelector("#pipeline")!.insertAdjacentHTML("beforeend", `<div class="card pose-report-card"><h3>Pose report inspection</h3>${issuesHtml(poseReportIssues, "Could not import pose report")}${body}</div><input id="pose-report-file" type="file" accept=".json,application/json" hidden />`);
}

function showNotice(message: string) {
  notice = message;
  let banner = document.querySelector<HTMLElement>(".notice");
  if (!banner) {
    banner = document.createElement("div");
    banner.className = "notice";
    banner.setAttribute("role", "status");
    document.querySelector(".page-heading")?.after(banner);
  }
  banner.textContent = message;
}

function enhanceSetup(value: Project) {
  const calibrationForm = document.querySelector<HTMLFormElement>("#calibration-form")!;
  calibrationForm.querySelector(".form-actions")!.insertAdjacentHTML("beforebegin", `<label class="field"><span>Distortion model</span><select name="distortionModel"><option value="" ${!value.calibration?.distortionModel ? "selected" : ""}>Legacy / unspecified</option><option value="opencv-radtan" ${value.calibration?.distortionModel === "opencv-radtan" ? "selected" : ""}>OpenCV radial-tangential</option></select></label><p class="setup-note">OpenCV radial-tangential needs 4, 5, or 8 coefficients. Unspecified legacy calibration remains valid but cannot prepare a pose run.</p>`);
  const boardForm = document.querySelector<HTMLFormElement>("#board-form")!;
  boardForm.querySelector(".setup-note")!.textContent = "The measured board rotates rigidly with the object. Marker corners use the board plane z=0; enter each marker's decoded TL, TR, BR, BL corner order.";
  boardForm.querySelector(".form-actions")!.insertAdjacentHTML("beforebegin", `
    <label class="field"><span>Dictionary</span><select name="dictionary"><option value="" ${!value.board?.dictionary ? "selected" : ""}>Legacy / unspecified</option><option value="DICT_4X4_50" ${value.board?.dictionary === "DICT_4X4_50" ? "selected" : ""}>ArUco DICT_4X4_50</option></select></label>
    <label class="field"><span>Marker geometry JSON</span><textarea name="markers" rows="9" spellcheck="false" placeholder='[{"id":0,"cornersMm":[[-10,-10,0],[10,-10,0],[10,10,0],[-10,10,0]]}]'>${escapeHtml(value.board?.markers ? JSON.stringify(value.board.markers, null, 2) : "")}</textarea></label>
    <div class="grid-generator"><span>Generate rectangular layout</span><div class="generator-fields"><label>Columns<input name="gridColumns" type="number" min="1" max="50" step="1" value="2"></label><label>Rows<input name="gridRows" type="number" min="1" max="50" step="1" value="2"></label><label>Gap (mm)<input name="gridGap" type="number" min="0" step="any" value="5"></label><label>First ID<input name="gridFirstId" type="number" min="0" max="49" step="1" value="0"></label></div><button id="generate-layout" class="button subtle" type="button">Fill marker JSON</button><p class="help">IDs increase left to right, then top to bottom. Board origin is the grid center; x points right and y down in this layout, with z=0. Check physical orientation and measured spacing before saving.</p></div>`);
}

function update(mutator: (value: Project) => void) {
  if (!project) return;
  clearPoseReport();
  mutator(project);
  notice = "Changes saved in this browser. Download JSON to share the project.";
  persist();
  render();
}

function bind() {
  document.querySelectorAll<HTMLButtonElement>(".recent-project").forEach((button) => button.addEventListener("click", () => {
    const id = button.dataset.id ?? "";
    const saved = Object.hasOwn(library, id) ? library[id] : undefined;
    if (saved) { project = structuredClone(saved); clearPoseReport(); importIssues = []; notice = `Opened ${saved.name}.`; persist(); render(); }
  }));
  document.querySelector<HTMLButtonElement>("#new-button")?.addEventListener("click", () => {
    project = newProject("Untitled scan"); clearPoseReport(); importIssues = []; notice = "New project created."; persist(); render();
  });
  document.querySelector<HTMLFormElement>("#create-form")?.addEventListener("submit", (event) => {
    event.preventDefault(); const form = event.currentTarget as HTMLFormElement;
    project = newProject(String(new FormData(form).get("name") ?? "")); clearPoseReport(); importIssues = []; notice = "New project created."; persist(); render();
  });
  document.querySelector<HTMLButtonElement>("#import-button")?.addEventListener("click", () => document.querySelector<HTMLInputElement>("#file-input")?.click());
  document.querySelector<HTMLInputElement>("#file-input")?.addEventListener("change", async (event) => {
    const file = (event.currentTarget as HTMLInputElement).files?.[0]; if (!file) return;
    if (file.size > 16 * 1024 * 1024) { importIssues = [{ path: "$", message: "Project JSON exceeds the 16 MiB import limit." }]; notice = `Could not import ${file.name}. The current project was kept.`; render(); return; }
    const parsed = parseProject(await file.text()); importIssues = parsed.issues;
    if (parsed.project) { project = parsed.project; clearPoseReport(); notice = `Imported ${file.name}. Image files are not included in JSON.`; persist(); }
    else notice = `Could not import ${file.name}. The current project was kept.`;
    render();
  });
  document.querySelector<HTMLButtonElement>("#download-button")?.addEventListener("click", () => {
    if (!project || validateProject(project).length) return;
    const blob = new Blob([JSON.stringify(project, null, 2) + "\n"], { type: "application/json" });
    const url = URL.createObjectURL(blob); const link = document.createElement("a");
    link.href = url; link.download = `${slug(project.name)}.crispmesh.json`; link.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  });
  document.querySelector<HTMLButtonElement>("#import-pose-report")?.addEventListener("click", () => document.querySelector<HTMLInputElement>("#pose-report-file")?.click());
  document.querySelector<HTMLInputElement>("#pose-report-file")?.addEventListener("change", async (event) => {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    const selected = project;
    const selectedRevision = inputRevision;
    if (!file || !selected) return;
    if (file.size > 16 * 1024 * 1024) { poseReportIssues = [{ path: "$", message: "Pose report exceeds the 16 MiB import limit." }]; render(); return; }
    let source: string;
    try { source = await file.text(); }
    catch { if (project === selected && inputRevision === selectedRevision) { poseReportIssues = [{ path: "$", message: "Could not read the pose report file." }]; render(); } return; }
    if (project !== selected || inputRevision !== selectedRevision) return;
    const parsed = parsePoseReport(source, selected);
    poseReportIssues = parsed.issues;
    if (parsed.report) { poseReport = parsed.report; poseReportName = file.name; notice = `Imported ${file.name} for inspection. Project stages were not changed.`; }
    render();
  });
  document.querySelector<HTMLButtonElement>("#import-sparse-report")?.addEventListener("click", () => document.querySelector<HTMLInputElement>("#sparse-report-file")?.click());
  document.querySelector<HTMLInputElement>("#sparse-report-file")?.addEventListener("change", async (event) => {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    const selected = project;
    const selectedRevision = inputRevision;
    const selectedImportRevision = ++sparseImportRevision;
    if (!file || !selected) return;
    if (file.size > 32 * 1024 * 1024) { sparseReportIssues = [{ path: "$", message: "Sparse report exceeds the 32 MiB import limit." }]; render(); return; }
    let source: string;
    try { source = await file.text(); }
    catch { if (project === selected && inputRevision === selectedRevision && sparseImportRevision === selectedImportRevision) { sparseReportIssues = [{ path: "$", message: "Could not read the sparse report file." }]; render(); } return; }
    if (project !== selected || inputRevision !== selectedRevision || sparseImportRevision !== selectedImportRevision) return;
    const parsed = parseSparseReport(source, selected);
    sparseReportIssues = parsed.issues;
    if (parsed.report) { sparseReport = parsed.report; sparseReportName = file.name; notice = `Imported ${file.name} for inspection. Project stages were not changed.`; }
    render();
  });
  document.querySelector<HTMLInputElement>("#project-name")?.addEventListener("change", (event) => update((value) => { value.name = (event.currentTarget as HTMLInputElement).value.trim(); }));
  document.querySelector<HTMLFormElement>("#add-image-form")?.addEventListener("submit", (event) => {
    event.preventDefault(); const data = new FormData(event.currentTarget as HTMLFormElement);
    const id = String(data.get("id") ?? "").trim(); const path = String(data.get("path") ?? "").trim(); const maskPath = String(data.get("maskPath") ?? "").trim();
    if (!id || !validRelativePath(path) || (maskPath && !validRelativePath(maskPath)) || project?.images.some((image) => image.id === id)) { notice = "Use a unique image ID and valid relative paths."; render(); return; }
    update((value) => { value.images.push({ id, path, ...(maskPath ? { maskPath } : {}) }); });
  });
  document.querySelectorAll<HTMLButtonElement>(".remove-image").forEach((button) => button.addEventListener("click", () => update((value) => { value.images.splice(Number(button.dataset.index), 1); })));
  document.querySelector<HTMLFormElement>("#calibration-form")?.addEventListener("submit", (event) => {
    event.preventDefault(); const data = new FormData(event.currentTarget as HTMLFormElement);
    const fields = Object.fromEntries(["width", "height", "fx", "fy", "cx", "cy"].map((key) => [key, Number(data.get(key))]));
    const raw = String(data.get("distortion") ?? "").trim();
    const parts = raw ? raw.split(",").map((part) => part.trim()) : [];
    const distortion = parts.map(Number);
    const model = String(data.get("distortionModel") ?? "");
    const next: Calibration = { ...project?.calibration, width: fields.width, height: fields.height, fx: fields.fx, fy: fields.fy, cx: fields.cx, cy: fields.cy, distortion };
    if (model) next.distortionModel = "opencv-radtan";
    else delete next.distortionModel;
    if (["width", "height", "fx", "fy", "cx", "cy"].some((key) => String(data.get(key) ?? "").trim() === "") || !Number.isInteger(next.width) || next.width <= 0 || !Number.isInteger(next.height) || next.height <= 0 || !Number.isFinite(next.fx) || next.fx <= 0 || !Number.isFinite(next.fy) || next.fy <= 0 || !Number.isFinite(next.cx) || !Number.isFinite(next.cy) || parts.some((part) => !part) || !distortion.every(Number.isFinite) || (model !== "" && model !== "opencv-radtan") || (model === "opencv-radtan" && ![4,5,8].includes(distortion.length))) { showNotice("Calibration needs positive dimensions and focal lengths, finite numbers, and 4, 5, or 8 coefficients for OpenCV radial-tangential."); return; }
    update((value) => { value.calibration = next; });
  });
  document.querySelector<HTMLButtonElement>("#remove-calibration")?.addEventListener("click", () => update((value) => { delete value.calibration; }));
  document.querySelector<HTMLFormElement>("#board-form")?.addEventListener("submit", (event) => {
    event.preventDefault(); const data = new FormData(event.currentTarget as HTMLFormElement);
    const type = String(data.get("type")); const markerSizeMm = Number(data.get("markerSizeMm"));
    const dictionary = String(data.get("dictionary") ?? "");
    const markerText = String(data.get("markers") ?? "").trim();
    let markers: unknown;
    try { markers = markerText ? JSON.parse(markerText) : undefined; }
    catch { showNotice("Marker geometry must be valid JSON."); return; }
    if ((type !== "aruco" && type !== "apriltag") || !Number.isFinite(markerSizeMm) || markerSizeMm <= 0 || (dictionary !== "" && dictionary !== "DICT_4X4_50")) { showNotice("Enter a positive marker size and supported dictionary."); return; }
    const next = { ...project?.board, type, markerSizeMm, rotatesWithObject: true, ...(dictionary ? { dictionary } : {}), ...(markers !== undefined ? { markers } : {}) } as Board;
    if (!dictionary) delete next.dictionary;
    if (markers === undefined) delete next.markers;
    const candidate = { ...project!, board: next };
    const boardIssues = validateProject(candidate).filter((issue) => issue.path.startsWith("board."));
    if (boardIssues.length) { showNotice(boardIssues.map((issue) => `${issue.path}: ${issue.message}`).join(" ")); return; }
    update((value) => { value.board = next; });
  });
  document.querySelector<HTMLButtonElement>("#generate-layout")?.addEventListener("click", () => {
    const form = document.querySelector<HTMLFormElement>("#board-form")!;
    const data = new FormData(form);
    try {
      const markers = generateBoardGrid(Number(data.get("gridColumns")), Number(data.get("gridRows")), Number(data.get("markerSizeMm")), Number(data.get("gridGap")), Number(data.get("gridFirstId")));
      (form.elements.namedItem("markers") as HTMLTextAreaElement).value = JSON.stringify(markers, null, 2);
      (form.elements.namedItem("dictionary") as HTMLSelectElement).value = "DICT_4X4_50";
      (form.elements.namedItem("type") as HTMLSelectElement).value = "aruco";
      showNotice("Grid filled. Confirm measurements and save the board.");
    } catch (error) { showNotice(error instanceof Error ? error.message : "Could not generate the layout."); }
  });
  document.querySelector<HTMLButtonElement>("#remove-board")?.addEventListener("click", () => update((value) => { delete value.board; }));
}

restore(); render();
if ("__TAURI_INTERNALS__" in window) {
  invoke<string>("desktop_capabilities").then((result) => { nativeStatus = result; render(); }).catch(() => { nativeStatus = "Desktop shell · capabilities unavailable"; render(); });
}
