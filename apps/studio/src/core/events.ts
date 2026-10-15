/**
 * The run event log of docs/ENGINE-CONTRACT.md, as far as a front end may rely on it.
 * Everything here is tolerant: unknown types, kinds and fields pass through untouched
 * and are ignored by the reducer.
 */

export const SCHEMA = "crispmesh_dense_events_v1";

/** One line of `events.jsonl`. Only `type` is guaranteed after parsing; the reducer checks the rest. */
export interface RunEvent {
  type: string;
  /** Line number in the log. Present over HTTP; assigned from the line index in replay. */
  seq: number;
  /** Unix seconds. */
  time: number;
  stage: string | null;
  [field: string]: unknown;
}

export interface ParsedLines {
  events: RunEvent[];
  /** Number of complete lines consumed: the reader's new position. */
  consumed: number;
}

/**
 * Parses the text of an event log (or a tail of it starting at line `firstSeq`).
 * A last line without a newline is still being written and is not consumed. A complete
 * line that is not a JSON object stops the reader there, like the reference reader does.
 */
export function parseEventLines(text: string, firstSeq = 0): ParsedLines {
  const events: RunEvent[] = [];
  let consumed = 0;
  let start = 0;
  for (;;) {
    const end = text.indexOf("\n", start);
    if (end < 0) break;
    const line = text.slice(start, end);
    start = end + 1;
    const event = normaliseEvent(safeParse(line), firstSeq + consumed);
    if (event === null) {
      if (line.trim() === "") {
        consumed += 1;
        continue;
      }
      break;
    }
    events.push(event);
    consumed += 1;
  }
  return { events, consumed };
}

function safeParse(line: string): unknown {
  try {
    return JSON.parse(line);
  } catch {
    return null;
  }
}

/** Accepts anything that is an object with a string `type`; fills in `seq`, `time` and `stage` defensively. */
export function normaliseEvent(raw: unknown, fallbackSeq: number): RunEvent | null {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return null;
  const record = raw as Record<string, unknown>;
  if (typeof record.type !== "string") return null;
  const seq = typeof record.seq === "number" && Number.isFinite(record.seq) ? record.seq : fallbackSeq;
  const time = typeof record.time === "number" && Number.isFinite(record.time) ? record.time : 0;
  const stage = typeof record.stage === "string" ? record.stage : null;
  return { ...record, type: record.type, seq, time, stage };
}

export function text(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

export function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}
