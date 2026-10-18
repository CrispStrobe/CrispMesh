import { useEffect, useState } from "preact/hooks";
import { humanise } from "../core/format";
import type { ReportRef, RunState } from "../core/reducer";
import { formatMetricValue, summariseReport, type ReportSummary } from "../core/reports";
import { describe, isAbort } from "../sources/transport";
import type { RunSource } from "../sources/types";
import { stageTitle } from "./stages";
import { t } from "../i18n";

interface Props {
  source: RunSource;
  run: RunState;
}

type Loaded = { state: "loading" } | { state: "ready"; json: unknown; summary: ReportSummary } | { state: "failed"; message: string };

function useReport(source: RunSource, report: ReportRef): Loaded {
  const [loaded, setLoaded] = useState<Loaded>({ state: "loading" });
  useEffect(() => {
    const abort = new AbortController();
    setLoaded({ state: "loading" });
    source
      .fetchJson(report.path, { signal: abort.signal })
      .then((json) => setLoaded({ state: "ready", json, summary: summariseReport(json) }))
      .catch((problem) => {
        if (!isAbort(problem) && !abort.signal.aborted) setLoaded({ state: "failed", message: describe(problem) });
      });
    return () => abort.abort();
  }, [source, report.path, report.seq]);
  return loaded;
}

function rawJson(json: unknown): string {
  try {
    const text = JSON.stringify(json, null, 2) ?? String(json);
    return text.length > 200_000 ? text.slice(0, 200_000) + "\n" + t("run.reports.shortened") : text;
  } catch {
    return String(json);
  }
}

function ReportCard({ source, report, optional }: { source: RunSource; report: ReportRef; optional?: boolean }) {
  const loaded = useReport(source, report);
  // A report nobody announced (looked for by convention) simply is not there when it fails.
  if (optional && loaded.state !== "ready") return null;
  return (
    <article class="report">
      <h3>{report.label}</h3>
      {loaded.state === "loading" && <p class="sub">{t("run.reports.loading")}</p>}
      {loaded.state === "failed" && (
        <p class="field-error" role="alert">
          {t("run.reports.readFailed", { error: loaded.message })}
        </p>
      )}
      {loaded.state === "ready" && (
        <>
          {loaded.summary.warnings.map((warning, index) => (
            <div class="notice warn" role="note" key={index}>
              <p>{warning}</p>
            </div>
          ))}
          {loaded.summary.kind === "unknown" && <p class="sub">{t("run.reports.unknownKind")}</p>}
          {loaded.summary.sections.map((section, index) => (
            <div class="report-section" key={index}>
              {section.title !== undefined && <h4>{section.title}</h4>}
              <dl class="facts">
                {section.rows.map((row) => (
                  <div key={row.label} class={row.tone !== undefined ? `tone-${row.tone}` : undefined}>
                    <dt>{row.label}</dt>
                    <dd>{row.value}</dd>
                  </div>
                ))}
              </dl>
            </div>
          ))}
          {loaded.summary.notes.length > 0 && (
            <ul class="report-notes">
              {loaded.summary.notes.map((note, index) => (
                <li key={index}>{note}</li>
              ))}
            </ul>
          )}
          <details class="raw" open={loaded.summary.kind === "unknown"}>
            <summary>{t("run.reports.fullJson")}</summary>
            <pre tabIndex={0}>{rawJson(loaded.json)}</pre>
          </details>
        </>
      )}
    </article>
  );
}

export function ReportsPanel({ source, run }: Props) {
  // The photo check is announced by a `report` event. Runs made before the engine did that
  // have check/result.json on disk without an event; only for those (check finished, nothing
  // announced) a live engine is asked for the file by its conventional path.
  const checkDone = run.stages.find((stage) => stage.name === "check")?.status === "done";
  const announced = run.reports.some((report) => report.path === "check/result.json");
  const photoCheck: ReportRef | null =
    source.kind !== "replay" && checkDone && !announced
      ? { seq: -1, path: "check/result.json", label: t("run.reports.photoCheck"), stage: "check" }
      : null;
  const configuration = run.configuration;

  return (
    <section class="panel reports-panel" aria-labelledby="numbers-heading">
      <div class="panel-head">
        <h2 id="numbers-heading">{t("run.reports.heading")}</h2>
      </div>

      {run.metrics.length === 0 && run.reports.length === 0 && (
        <p class="sub">
          {run.status === "running" || run.status === "waiting" ? t("run.reports.soon") : t("run.reports.none")}
        </p>
      )}

      {run.metrics.length > 0 && (
        <dl class="facts metrics" aria-label={t("run.reports.metrics")}>
          {run.metrics.map((metric) => (
            <div key={metric.name}>
              <dt>
                {humanise(metric.name)}
                {metric.stage !== null && <span class="fact-stage">{stageTitle(metric.stage)}</span>}
              </dt>
              <dd class={typeof metric.value === "number" ? "metric-number" : undefined}>{formatMetricValue(metric.value)}</dd>
            </div>
          ))}
        </dl>
      )}

      {run.reports.map((report) => (
        <ReportCard key={report.path} source={source} report={report} />
      ))}
      {photoCheck !== null && <ReportCard source={source} report={photoCheck} optional />}

      {configuration !== undefined && (
        <details class="raw">
          <summary>{t("run.reports.settings")}</summary>
          <dl class="facts compact">
            {run.inputs !== undefined && (
              <div>
                <dt>{t("run.reports.inputs")}</dt>
                <dd class="mono wrap">{run.inputs}</dd>
              </div>
            )}
            {Object.entries(configuration).map(([name, value]) => (
              <div key={name}>
                <dt>{humanise(name)}</dt>
                <dd>{Array.isArray(value) ? value.join(", ") : typeof value === "object" ? JSON.stringify(value) : String(value)}</dd>
              </div>
            ))}
          </dl>
        </details>
      )}
    </section>
  );
}
