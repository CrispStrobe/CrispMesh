/**
 * Logic of the generated settings form: text <-> typed values, what changed against the
 * defaults, and where a server-side validation message belongs. No DOM.
 */

import { t } from "../i18n";

export type SettingValue = boolean | number | number[] | string;

export interface SettingSpec {
  name: string;
  group: string;
  meaning: string;
  /** `boolean`, `integer`, `number`, `integer_list`, `number_list`; anything else is edited as raw JSON. */
  kind: string;
  default: unknown;
}

export interface SettingGroup {
  group: string;
  settings: SettingSpec[];
}

/** What the form holds per setting: a checkbox state or the text of an input. */
export type FormValues = Record<string, string | boolean>;

/** Reads `GET /api/settings`, dropping rows this front end cannot use. */
export function parseSettingsSchema(payload: unknown): SettingSpec[] {
  const rows = (payload as { settings?: unknown } | null)?.settings;
  if (!Array.isArray(rows)) return [];
  const specs: SettingSpec[] = [];
  for (const row of rows) {
    if (typeof row !== "object" || row === null) continue;
    const record = row as Record<string, unknown>;
    if (typeof record.name !== "string" || record.name === "") continue;
    specs.push({
      name: record.name,
      group: typeof record.group === "string" && record.group !== "" ? record.group : "Other",
      meaning: typeof record.meaning === "string" ? record.meaning : "",
      kind: typeof record.kind === "string" ? record.kind : "unknown",
      default: record.default,
    });
  }
  return specs;
}

/** Groups in first-appearance order, settings in declaration order. */
export function groupSettings(specs: readonly SettingSpec[]): SettingGroup[] {
  const groups: SettingGroup[] = [];
  for (const spec of specs) {
    let group = groups.find((candidate) => candidate.group === spec.group);
    if (group === undefined) groups.push((group = { group: spec.group, settings: [] }));
    group.settings.push(spec);
  }
  return groups;
}

export function defaultFormValue(spec: SettingSpec): string | boolean {
  if (spec.kind === "boolean") return spec.default === true;
  if (spec.kind === "integer_list" || spec.kind === "number_list") {
    return Array.isArray(spec.default) ? spec.default.map(String).join(", ") : "";
  }
  if (spec.kind === "integer" || spec.kind === "number") return spec.default === undefined ? "" : String(spec.default);
  return spec.default === undefined ? "" : JSON.stringify(spec.default);
}

export function defaultFormValues(specs: readonly SettingSpec[]): FormValues {
  const values: FormValues = {};
  for (const spec of specs) values[spec.name] = defaultFormValue(spec);
  return values;
}

export type Parsed = { ok: true; value: SettingValue } | { ok: false; error: string };

const INTEGER = /^[+-]?\d+$/;

function parseNumber(raw: string, integer: boolean): number | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  if (integer) return INTEGER.test(trimmed) ? Number(trimmed) : null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : null;
}

/** Turns what the user typed into the value the engine expects, or says what is wrong with it. */
export function parseFormValue(spec: SettingSpec, raw: string | boolean): Parsed {
  if (spec.kind === "boolean") return { ok: true, value: raw === true || raw === "true" };
  const textValue = String(raw);
  switch (spec.kind) {
    case "integer":
    case "number": {
      const value = parseNumber(textValue, spec.kind === "integer");
      if (value === null) {
        return { ok: false, error: spec.kind === "integer" ? t("inputs.option.needsWhole") : t("inputs.option.needsNumber") };
      }
      return { ok: true, value };
    }
    case "integer_list":
    case "number_list": {
      const integer = spec.kind === "integer_list";
      const parts = textValue.split(/[,\s]+/).filter((part) => part !== "");
      if (parts.length === 0) return { ok: false, error: t("inputs.setting.needsOne") };
      const values: number[] = [];
      for (const part of parts) {
        const value = parseNumber(part, integer);
        if (value === null) {
          return {
            ok: false,
            error: t(integer ? "inputs.setting.notWhole" : "inputs.setting.notNumber", { part }),
          };
        }
        values.push(value);
      }
      return { ok: true, value: values };
    }
    default:
      try {
        return { ok: true, value: JSON.parse(textValue) as SettingValue };
      } catch {
        return { ok: false, error: t("inputs.setting.needsJson") };
      }
  }
}

function sameValue(a: unknown, b: unknown): boolean {
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((item, i) => sameValue(item, b[i]));
  if (typeof a === "object" && a !== null && typeof b === "object" && b !== null) {
    return JSON.stringify(a) === JSON.stringify(b);
  }
  return a === b;
}

export interface SettingsDiff {
  /** Only the settings whose value differs from the default: this is what gets sent. */
  changed: Record<string, SettingValue>;
  /** Client-side problems, by setting name. */
  errors: Record<string, string>;
}

export function diffSettings(specs: readonly SettingSpec[], form: FormValues): SettingsDiff {
  const changed: Record<string, SettingValue> = {};
  const errors: Record<string, string> = {};
  for (const spec of specs) {
    const raw = form[spec.name];
    if (raw === undefined) continue;
    const parsed = parseFormValue(spec, raw);
    if (!parsed.ok) {
      // An untouched field never blocks: a default this form cannot express is simply not sent.
      if (raw !== defaultFormValue(spec)) errors[spec.name] = parsed.error;
      continue;
    }
    if (!sameValue(parsed.value, spec.default)) changed[spec.name] = parsed.value;
  }
  return { changed, errors };
}

export function isChanged(spec: SettingSpec, raw: string | boolean | undefined): boolean {
  if (raw === undefined) return false;
  const parsed = parseFormValue(spec, raw);
  return !parsed.ok ? raw !== defaultFormValue(spec) : !sameValue(parsed.value, spec.default);
}

export interface ServerErrorPlacement {
  /** Field names (settings, or `inputs` / `reference` / ...) the message is about; empty if it cannot be placed. */
  fields: string[];
  message: string;
}

/**
 * The engine answers 400 with one sentence. It names the offending setting(s) in it
 * ("dense configuration: sizes must increase", "inputs must be an existing path ..."),
 * so the message is shown at those fields and always once at the form as well.
 */
export function placeServerError(message: string, fieldNames: readonly string[]): ServerErrorPlacement {
  const bare = message.replace(/^dense configuration:\s*/, "");
  // Shown as a sentence; the engine writes it lower-case and without a full stop.
  const clean = bare.charAt(0).toUpperCase() + bare.slice(1) + (/[.!?]$/.test(bare) || bare.length > 200 ? "" : ".");
  // "unknown setting 'x=1'; known: a, b, c" lists every name; that is not about any one field.
  const searchable = message.split("; known:")[0] ?? message;
  const fields = fieldNames.filter((name) => new RegExp(`(^|[^A-Za-z0-9_])${escapeRegExp(name)}([^A-Za-z0-9_]|$)`).test(searchable));
  const leading = /^([a-z_]+) (must|needs) /.exec(bare)?.[1];
  if (leading !== undefined && fields.includes(leading)) return { fields: [leading], message: clean };
  return { fields, message: clean };
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export interface StartRunForm {
  name: string;
  inputs: string;
  reference: string;
  device: string;
}

/** Body of `POST /api/runs`: empty optional fields are left out, settings carry only changed values. */
export function startRunBody(form: StartRunForm, changed: Record<string, SettingValue>): Record<string, unknown> {
  const body: Record<string, unknown> = { inputs: form.inputs.trim() };
  if (form.name.trim() !== "") body.name = form.name.trim();
  if (form.device.trim() !== "") body.device = form.device.trim();
  if (form.reference.trim() !== "") body.reference = form.reference.trim();
  if (Object.keys(changed).length > 0) body.settings = changed;
  return body;
}
