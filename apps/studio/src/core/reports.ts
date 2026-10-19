/**
 * Turns the JSON behind a `report` artifact into a few rows worth reading. Shapes are
 * recognised by their fields; anything unrecognised is returned as `unknown` and the
 * view shows the raw JSON instead. Never throws.
 */

import { formatCount, formatNumber, formatPercent } from "./format";
import { localDecimal, t } from "../i18n";

export type Tone = "good" | "bad" | "neutral";

export interface ReportRow {
  label: string;
  value: string;
  tone?: Tone;
}

export interface ReportSection {
  title?: string;
  rows: ReportRow[];
}

export interface ReportSummary {
  kind: "mesh" | "photo_check" | "camera_recovery" | "scan" | "unknown";
  sections: ReportSection[];
  warnings: string[];
  notes: string[];
}

type Json = Record<string, unknown>;

function record(value: unknown): Json | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Json) : undefined;
}

function num(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

export function summariseReport(json: unknown): ReportSummary {
  const root = record(json);
  const empty: ReportSummary = { kind: "unknown", sections: [], warnings: [], notes: [] };
  if (root === undefined) return empty;
  try {
    if (root.schema === "scan_evaluate_v1" || record(record(root.metrics)?.all)?.thresholds !== undefined) {
      return scan(root);
    }
    if (record(root.silhouette_iou_input_masks) !== undefined) return photoCheck(root);
    if (typeof root.closed === "boolean" && num(root.triangles) !== undefined) return mesh(root);
    const cameras = cameraRecovery(root);
    if (cameras) return cameras;
  } catch {
    // fall through to the raw view
  }
  return { ...empty, warnings: strings(root.warnings) };
}

function cameraRecovery(root: Json): ReportSummary | undefined {
  const minimumCoverage = num(root.minimum_registered_fraction);
  const minimumSupport = num(root.minimum_sparse_mask_support);
  const minimumGeometry = num(root.minimum_base_mask_iou);
  if (root.minimum_base_mask_iou !== undefined && (minimumGeometry === undefined || minimumGeometry < 0 || minimumGeometry > 1)) return undefined;
  if (!Array.isArray(root.candidates) || typeof root.selected !== "string" || minimumCoverage === undefined || minimumSupport === undefined ||
      minimumCoverage < 0 || minimumCoverage > 1 || minimumSupport < 0 || minimumSupport > 1) return undefined;
  const candidates = root.candidates.map(record).filter((row): row is Json => row !== undefined);
  if (candidates.length !== root.candidates.length || !candidates.some(row => row.path === root.selected) || candidates.some(row => {
    const count = num(row.registered), total = num(row.accepted), support = num(row.sparse_mask_support);
    return typeof row.path !== "string" || count === undefined || total === undefined || !Number.isInteger(count) || !Number.isInteger(total) ||
      count < 0 || total < 1 || count > total || (row.sparse_mask_support != null && (support === undefined || support < 0 || support > 1));
  })) return undefined;
  const label = (path: string) => path === "cameras" ? t("run.report.cameraTight") :
    path === "cameras-context" ? t("run.report.cameraContext") : path === "cameras-hybrid" ? t("run.report.cameraHybrid") : path;
  const sections: ReportSection[] = [{ rows: [{ label: t("run.report.cameraSelected"), value: label(root.selected) }] }];
  for (const row of candidates) {
    const count = row.registered as number, total = row.accepted as number, support = num(row.sparse_mask_support);
    const coveragePassed = count >= Math.ceil(total * minimumCoverage), supportPassed = support !== undefined && support >= minimumSupport;
    const dropped = num(row.removed_weak_views);
    const geometry = num(row.base_mask_iou);
    const geometryPassed = minimumGeometry === undefined || (geometry !== undefined && geometry >= minimumGeometry);
    sections.push({ title: label(row.path as string), rows: [
      { label: t("run.report.cameraCoverage"), value: t("run.report.viewsOf", { views: formatCount(count), total: formatCount(total) }), tone: coveragePassed ? "good" : "bad" },
      { label: t("run.report.cameraSupport"), value: support === undefined ? "—" : formatPercent(support, 1), tone: supportPassed ? "good" : "bad" },
      ...(minimumGeometry !== undefined ? [{ label: t("run.report.cameraGeometry"), value: geometry === undefined ? "—" : formatPercent(geometry, 1), tone: geometryPassed ? "good" as const : "bad" as const }] : []),
      ...(dropped !== undefined && Number.isInteger(dropped) && dropped > 0 ? [{ label: t("run.report.cameraDropped"), value: formatCount(dropped) }] : []),
      { label: t("run.report.cameraGate"), value: yesNo(coveragePassed && supportPassed && geometryPassed), tone: coveragePassed && supportPassed && geometryPassed ? "good" : "bad" },
    ] });
  }
  return { kind: "camera_recovery", sections, warnings: [], notes: [t("run.report.cameraNote")] };
}

function mesh(root: Json): ReportSummary {
  const rows: ReportRow[] = [];
  const count = (label: string, key: string, badIfPositive = false) => {
    const value = num(root[key]);
    if (value === undefined) return;
    rows.push({ label, value: formatCount(value), tone: badIfPositive ? (value > 0 ? "bad" : "good") : undefined });
  };
  count(t("run.report.triangles"), "triangles");
  count(t("run.report.vertices"), "vertices");
  rows.push({ label: t("run.report.closed"), value: yesNo(root.closed === true), tone: root.closed === true ? "good" : "bad" });
  count(t("run.report.genus"), "genus");
  count(t("run.report.boundaryEdges"), "boundary_edges", true);
  count(t("run.report.nonManifoldEdges"), "nonmanifold_edges", true);
  count(t("run.report.components"), "components");
  const fraction = (label: string, key: string) => {
    const value = num(root[key]);
    if (value !== undefined) rows.push({ label, value: formatPercent(value) });
  };
  fraction(t("run.report.hullMeasured"), "observed_hull_fraction");
  fraction(t("run.report.hullFilled"), "extrapolated_hull_fraction");
  const notes: string[] = [];
  if (root.physical_scale_established === false) notes.push(t("run.report.noScale"));
  return { kind: "mesh", sections: [{ rows }], warnings: strings(root.warnings), notes };
}

function photoCheck(root: Json): ReportSummary {
  const sections: ReportSection[] = [];
  const block = (title: string, key: string) => {
    const values = record(root[key]);
    if (values === undefined) return;
    const rows: ReportRow[] = [];
    for (const [label, field] of [
      [t("run.report.median"), "median"],
      [t("run.report.lowest"), "minimum"],
      [t("run.report.highest"), "maximum"],
    ] as const) {
      const value = num(values[field]);
      if (value !== undefined) rows.push({ label, value: localDecimal(value.toFixed(3)) });
    }
    if (typeof values.worst_view === "string") rows.push({ label: t("run.report.weakestView"), value: values.worst_view });
    sections.push({ title, rows });
  };
  block(t("run.report.overlapInput"), "silhouette_iou_input_masks");
  block(t("run.report.overlapRepaired"), "silhouette_iou_repaired_masks");
  const views = num(root.views_checked);
  const total = num(root.views);
  if (views !== undefined) {
    sections.push({ rows: [{ label: t("run.report.viewsChecked"), value: total === undefined ? String(views) : t("run.report.viewsOf", { views, total }) }] });
  }
  const notes = typeof root.note === "string" ? [capitalise(root.note)] : [];
  return { kind: "photo_check", sections, warnings: strings(root.warnings), notes };
}

function scan(root: Json): ReportSummary {
  const metrics = record(root.metrics) ?? {};
  const sections: ReportSection[] = [];
  const titles: Record<string, string> = { all: t("run.report.wholeSurface"), above_margin: t("run.report.aboveSupport") };
  for (const key of ["all", "above_margin"]) {
    const block = record(metrics[key]);
    if (block === undefined) continue;
    const rows: ReportRow[] = [];
    const thresholds = record(block.thresholds) ?? {};
    const keys = Object.keys(thresholds).sort((a, b) => Number(a) - Number(b));
    for (const threshold of keys) {
      const entry = record(thresholds[threshold]);
      const f1 = num(entry?.f1);
      const fraction = num(entry?.fraction_of_diagonal) ?? Number(threshold);
      if (f1 === undefined || !Number.isFinite(fraction)) continue;
      rows.push({ label: t("run.report.f1At", { threshold: trimPercent(fraction) }), value: localDecimal(f1.toFixed(3)) });
    }
    const distance = (label: string, field: string) => {
      const summary = record(record(block[field])?.fraction_of_diagonal);
      const median = num(summary?.median);
      const p90 = num(summary?.p90);
      if (median === undefined) return;
      rows.push({
        label,
        value: p90 === undefined ? formatPercent(median, 2) : t("run.report.withP90", { median: formatPercent(median, 2), p90: formatPercent(p90, 2) }),
      });
    };
    distance(t("run.report.accuracy"), "accuracy_mesh_to_reference");
    distance(t("run.report.completeness"), "completeness_reference_to_mesh");
    if (rows.length > 0) sections.push({ title: titles[key], rows });
  }
  const handedness = record(record(root.alignment)?.handedness);
  if (typeof handedness?.scored === "string") {
    sections.push({ rows: [{ label: t("run.report.handedness"), value: handedness.scored }] });
  }
  const notes = [t("run.report.distancesNote"), ...strings(root.notes)];
  return {
    kind: sections.length > 0 ? "scan" : "unknown",
    sections,
    warnings: strings(root.warnings),
    notes,
  };
}

function trimPercent(fraction: number): string {
  return localDecimal(`${Number((fraction * 100).toFixed(3))}%`);
}

function yesNo(value: boolean): string {
  return value ? t("run.report.yes") : t("run.report.no");
}

function capitalise(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

/** A metric value for the compact list. */
export function formatMetricValue(value: number | string | boolean): string {
  if (typeof value === "number") return formatNumber(value);
  if (typeof value === "boolean") return yesNo(value);
  return value;
}
