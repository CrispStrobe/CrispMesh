import { useEffect, useState } from "preact/hooks";
import { formatClock, formatPercent, humanise } from "../core/format";
import { t, tParts, type MessageKey } from "../i18n";
import { describe, isAbort } from "../sources/transport";
import type { Engine, EngineHealth, RunSummary } from "../sources/types";
import { Icon } from "./icons";
import { stageTitle } from "./stages";

const REFRESH_MS = 3000;

export function StatusBadge({ status }: { status: string }) {
  const known = ["running", "complete", "failed", "cancelled", "waiting"].includes(status) ? status : "unknown";
  return <span class={`badge status-${known}`}>{known === "unknown" ? humanise(status) : t(`run.status.${known}` as MessageKey)}</span>;
}

export function ProgressBar({ fraction, label, active }: { fraction: number; label: string; active?: boolean }) {
  const percent = Math.round(Math.min(1, Math.max(0, fraction)) * 100);
  return (
    <div
      class={`progress${active ? " active" : ""}`}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
    >
      <div class="progress-fill" style={{ width: `${percent}%` }} />
    </div>
  );
}

export function Runs({ engine, settingsHref }: { engine: Engine; settingsHref?: string }) {
  const [runs, setRuns] = useState<RunSummary[] | null>(null);
  const [health, setHealth] = useState<EngineHealth | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    engine
      .health(abort.signal)
      .then(setHealth)
      .catch(() => undefined);
    const refresh = async () => {
      try {
        setRuns(await engine.listRuns(abort.signal));
        setError("");
      } catch (problem) {
        if (isAbort(problem)) return;
        setError(describe(problem));
      }
      if (!abort.signal.aborted) timer = setTimeout(refresh, document.hidden ? REFRESH_MS * 3 : REFRESH_MS);
    };
    void refresh();
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, [engine]);

  const canStart = health?.canStartRuns ?? false;

  return (
    <section class="page">
      <div class="page-head">
        <div>
          <h1>{t("run.list.title")}</h1>
          <p class="sub">
            {tParts("run.list.engine", { name: <span class="mono">{engine.label}</span> })}
            {health?.device !== undefined && <> · {t("run.list.defaultDevice", { device: health.device })}</>}
            {settingsHref !== undefined && (
              <>
                {" · "}
                <a href={settingsHref}>{t("run.list.engineSettings")}</a>
              </>
            )}
          </p>
        </div>
        {canStart ? (
          <a class="button primary" href="#/engine/new">
            <Icon name="plus" /> {t("run.list.newRun")}
          </a>
        ) : (
          health !== null && <span class="sub">{t("run.list.cannotStart")}</span>
        )}
      </div>

      {error !== "" && (
        <div class="notice bad" role="alert">
          <p>
            <strong>{t("run.list.notAnswering")}</strong> {t("run.list.tryingAgain", { error })}
          </p>
        </div>
      )}

      {runs === null && error === "" && <p class="sub">{t("run.list.loading")}</p>}

      {runs !== null && runs.length === 0 && (
        <div class="empty">
          <p>{t("run.list.empty")}</p>
          {canStart && (
            <a class="button primary" href="#/engine/new">
              {t("run.list.startFirst")}
            </a>
          )}
        </div>
      )}

      {runs !== null && runs.length > 0 && (
        <ul class="run-list">
          {runs.map((run) => (
            <li key={run.id}>
              <a class="run-row" href={`#/engine/run/${encodeURIComponent(run.id)}`}>
                <span class="run-name">{run.id}</span>
                <StatusBadge status={run.status} />
                <span class="run-stage">
                  {run.status === "running" && run.stage !== null && (
                    <>
                      <span class="run-stage-name">
                        {stageTitle(run.stage)} · {formatPercent(run.stageFraction, 0)}
                      </span>
                      <ProgressBar fraction={run.stageFraction} label={t("run.progressLabel", { stage: stageTitle(run.stage) })} active />
                    </>
                  )}
                  {run.status === "running" && run.stage === null && <span class="sub">{t("run.list.starting")}</span>}
                  {(run.status === "failed" || run.status === "cancelled") && run.stage !== null && (
                    <span class="sub">{t("run.list.stoppedIn", { stage: stageTitle(run.stage) })}</span>
                  )}
                  {run.status === "complete" && <span class="sub">{t("run.list.events", { count: run.events })}</span>}
                </span>
                <span class="run-started">{run.started !== null ? formatClock(run.started) : ""}</span>
              </a>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
