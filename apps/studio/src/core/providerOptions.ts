/**
 * Tuning options of the providers of the photos start, as the engine describes them
 * (`crispmesh-dense run --describe`: flag, kind, default, choices, meaning), and how their
 * values become the option words of the photos stage. Nothing about any particular
 * provider is written here. No DOM.
 *
 * Options that say where a program or a file is never appear in the form: an engine
 * leaves them out, and one that came through anyway is dropped here.
 */

export type OptionKind = "text" | "integer" | "number" | "choice" | "switch";

export interface OptionSpec {
  /** The flag without dashes, e.g. `turntable-span`. Also the key of its value in the form. */
  name: string;
  /** What it means, in the engine's words. */
  label: string;
  kind: OptionKind;
  /** The engine's default as it would be typed ("on"/"off" for a switch; "" when it has none). A value equal to it is not sent. */
  default: string;
  choices: string[];
  /** May be given several times: the value is split at spaces into one `--flag word` each. */
  repeated: boolean;
}

/** A path that belongs to one provider: data of the run (masks to import, a marker mat), with a field of its own in the start request. */
export interface PathInput {
  /** Key in the start request. */
  key: string;
  label: string;
  kind: "folder" | "file";
  help?: string;
}

const KINDS: OptionKind[] = ["text", "integer", "number", "choice", "switch"];

function rows(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.filter((row): row is Record<string, unknown> => typeof row === "object" && row !== null) : [];
}

/** Reads a list of options given as JSON; options of a kind the form has no field for (paths, programs) are left out. */
export function parseOptionSpecs(json: unknown): OptionSpec[] {
  const specs: OptionSpec[] = [];
  for (const row of rows(json)) {
    const name = typeof row.name === "string" && row.name !== "" ? row.name : typeof row.flag === "string" ? row.flag.replace(/^--/, "") : "";
    const kind = KINDS.find((known) => known === row.kind);
    if (name === "" || kind === undefined || specs.some((spec) => spec.name === name)) continue;
    const fallback = row.default;
    specs.push({
      name,
      label: typeof row.meaning === "string" && row.meaning !== "" ? row.meaning : name,
      kind,
      default: kind === "switch" ? (fallback === true || fallback === "on" ? "on" : "off") : fallback === null || fallback === undefined ? "" : String(fallback),
      choices: Array.isArray(row.choices) ? row.choices.filter((choice): choice is string => typeof choice === "string") : [],
      repeated: row.repeated === true,
    });
  }
  return specs;
}

export function parsePathInputs(json: unknown): PathInput[] {
  const inputs: PathInput[] = [];
  for (const row of rows(json)) {
    if (typeof row.key !== "string" || row.key === "") continue;
    inputs.push({
      key: row.key,
      label: typeof row.label === "string" && row.label !== "" ? row.label : row.key,
      kind: row.kind === "file" ? "file" : "folder",
      help: typeof row.help === "string" && row.help !== "" ? row.help : undefined,
    });
  }
  return inputs;
}

export type OptionValues = Record<string, string | boolean>;

/** A value as text, the way the default is written. */
export function normalValue(spec: OptionSpec, raw: string | boolean | undefined): string {
  if (raw === undefined) return spec.default;
  if (spec.kind === "switch") return raw === true || raw === "on" ? "on" : "off";
  return String(raw).trim();
}

function same(spec: OptionSpec, value: string): boolean {
  if (value === spec.default) return true;
  // 0.50 and 0.5 are the same number.
  return (spec.kind === "number" || spec.kind === "integer") && value !== "" && spec.default !== "" && Number(value) === Number(spec.default);
}

/** Whether a value differs from the engine's default. */
export function isChanged(spec: OptionSpec, raw: string | boolean | undefined): boolean {
  return !same(spec, normalValue(spec, raw));
}

/** What is wrong with a value, or undefined. The engine checks again; this only catches typing slips early. */
export function optionError(spec: OptionSpec, raw: string | boolean | undefined): string | undefined {
  const value = normalValue(spec, raw);
  if (same(spec, value) || spec.kind === "switch") return undefined;
  if (value === "") return spec.repeated ? undefined : "Needs a value.";
  if (spec.kind === "integer" && !/^[+-]?\d+$/.test(value)) return "Needs a whole number.";
  if (spec.kind === "number" && !Number.isFinite(Number(value))) return "Needs a number.";
  if (spec.kind === "choice" && !spec.choices.includes(value)) return `Must be one of: ${spec.choices.join(", ")}.`;
  if (!spec.repeated && value.startsWith("--")) return "A value cannot start with two dashes.";
  return undefined;
}

/** The option words for these options: only values that differ from the defaults, in the given order, each flag once. */
export function optionTokens(specs: OptionSpec[], values: OptionValues): string[] {
  const tokens: string[] = [];
  const seen = new Set<string>();
  for (const spec of specs) {
    if (seen.has(spec.name)) continue;
    seen.add(spec.name);
    const value = normalValue(spec, values[spec.name]);
    if (same(spec, value) || optionError(spec, values[spec.name]) !== undefined) continue;
    if (spec.kind === "switch") tokens.push(value === "on" ? `--${spec.name}` : `--no-${spec.name}`);
    else if (spec.repeated) for (const word of value.split(/\s+/).filter((part) => part !== "")) tokens.push(`--${spec.name}`, word);
    else tokens.push(`--${spec.name}`, value);
  }
  return tokens;
}

/** Problems by option name. */
export function optionErrors(specs: OptionSpec[], values: OptionValues): Record<string, string> {
  const errors: Record<string, string> = {};
  for (const spec of specs) {
    const problem = optionError(spec, values[spec.name]);
    if (problem !== undefined) errors[spec.name] = problem;
  }
  return errors;
}
