import { useEffect, useRef, useState } from "preact/hooks";
import { formatDuration, formatPercent } from "../core/format";
import { language, t, tParts, type MessageKey } from "../i18n";
import { STAGE_TEXT, stageTitle } from "./stages";
import { stageElapsed, type RunState, type StageState } from "../core/reducer";
import { REPLAY_SPEEDS, type ReplaySnapshot } from "../core/replay";
import { RunStore, type RunSnapshot } from "../sources/runStore";
import { describe } from "../sources/transport";
import type { LinkStatus, ReplayControls, RunSource } from "../sources/types";
import { canDownload, canShareFiles, downloadFile, shareFile } from "./download";
import { Gallery } from "./gallery";
import { InspectionPanel } from "./inspection";
import { Icon } from "./icons";
import { TexturePreview } from "./texturePreview";
import { MeshPanel } from "./meshPanel";
import { savePrefs, speedToPref } from "./prefs";
import { ReportsPanel } from "./reportsPanel";
import { ProgressBar, StatusBadge } from "./runs";

interface Props {
  source: RunSource;
  heading: string;
  backHref: string;
  /** Changes whenever the effective theme changes, so the 3D view can re-read its colours. */
  themeTick: number;
}

/** A running run with no event for this long gets a "no news" note. Matching one level can take minutes. */
const QUIET_AFTER_SECONDS = 180;

export function RunView({ source, heading, backHref, themeTick }: Props) {
  const [snapshot, setSnapshot] = useState<RunSnapshot | null>(null);
  const [now, setNow] = useState(() => source.now());
  const [downloadError, setDownloadError] = useState("");
  const [cancel, setCancel] = useState<{ state: "idle" | "asking" | "sent"; error?: string }>({ state: "idle" });

  useEffect(() => {
    const store = new RunStore(source);
    setSnapshot(store.get());
    const detach = store.subscribe(setSnapshot);
    source.start();
    return () => {
      detach();
      source.stop();
      store.dispose();
    };
  }, [source]);

  const running = snapshot?.run.status === "running";
  useEffect(() => {
    setNow(source.now());
    if (!running) return;
    const timer = setInterval(() => setNow(source.now()), 500);
    return () => clearInterval(timer);
  }, [running, source, snapshot?.run.eventCount]);

  if (snapshot === null) return null;
  const { run, link } = snapshot;
  // The run's own textured mesh first (the preview shows it), then the neural surface's and the splats'.
  const downloads = [...run.downloads].sort((a, b) => Number(a.stage === "surface" || a.stage === "splats") - Number(b.stage === "surface" || b.stage === "splats"));

  const requestCancel = async () => {
    if (source.cancel === undefined) return;
    setCancel({ state: "asking" });
    try {
      await source.cancel();
      setCancel({ state: "sent" });
    } catch (problem) {
      setCancel({ state: "idle", error: describe(problem) });
    }
  };

  // A live run that has said nothing for a while: worth telling, because a driver that dies
  // outside a stage leaves no `run_finished` behind.
  const quietFor =
    source.kind !== "replay" && running && link.state === "live" && run.lastTime !== undefined
      ? Math.floor((now - run.lastTime) / 10) * 10
      : undefined;
  const total = run.seconds ?? (run.startedAt !== undefined && running ? Math.max(0, now - run.startedAt) : undefined);

  return (
    <section class={`page run-view${source.replay !== undefined ? " has-replay-bar" : ""}`}>
      <div class="page-head">
        <div class="run-title">
          <a class="back-link" href={backHref}>
            <Icon name="back" size={16} /> {source.kind === "replay" ? t("run.view.backConnection") : t("run.view.backRuns")}
          </a>
          <h1>{heading}</h1>
          <p class="sub run-meta">
            <StatusBadge status={run.status} />
            {total !== undefined && <span>{formatDuration(total)}</span>}
            {run.device !== undefined && <span>{t("run.view.onDevice", { device: run.device })}</span>}
            {source.kind === "replay" && <span class="tag">{t("run.view.recording")}</span>}
            {source.memoryNote?.() !== undefined && <span class="memory-note">{source.memoryNote()}</span>}
          </p>
          <Attribution source={source} />
        </div>
        {source.cancel !== undefined && running && (
          <button
            type="button"
            class="button danger"
            onClick={requestCancel}
            disabled={cancel.state !== "idle"}
            aria-describedby={cancel.error !== undefined ? "cancel-error" : undefined}
          >
            <Icon name="stop" size={16} />
            {cancel.state === "idle" ? t("run.view.cancel") : cancel.state === "asking" ? t("run.view.cancelling") : t("run.view.stopping")}
          </button>
        )}
      </div>

      <LinkNotice link={link} />
      {cancel.error !== undefined && (
        <div class="notice bad" role="alert" id="cancel-error">
          <p>
            <strong>{t("run.view.cancelFailed")}</strong> {cancel.error}
          </p>
        </div>
      )}
      {run.schemaUnknown && (
        <div class="notice warn" role="status">
          <p>{t("run.view.newerSchema", { schema: run.schema ?? "" })}</p>
        </div>
      )}
      {run.errors.map((error) =>
        run.status === "cancelled" ? (
          <div class="notice warn" role="status" key={error.seq}>
            <p>
              {error.stage !== null
                ? tParts("run.view.cancelledDuring", {
                    cancelled: <strong>{t("run.view.wasCancelled")}</strong>,
                    stage: language() === "en" ? stageTitle(error.stage).toLowerCase() : stageTitle(error.stage),
                  })
                : tParts("run.view.cancelledPlain", { cancelled: <strong>{t("run.view.wasCancelled")}</strong> })}
            </p>
            <details class="raw">
              <summary>{t("run.view.lastOutput")}</summary>
              <pre class="error-text" tabIndex={0}>
                {error.message}
              </pre>
            </details>
          </div>
        ) : (
          <div class="notice bad" role="alert" key={error.seq}>
            <p>
              <strong>{error.stage !== null ? t("run.view.stageFailed", { stage: stageTitle(error.stage) }) : t("run.view.runFailed")}</strong>
            </p>
            <pre class="error-text" tabIndex={0}>
              {error.message}
            </pre>
          </div>
        ),
      )}
      {quietFor !== undefined && quietFor > QUIET_AFTER_SECONDS && (
        <div class="notice" role="status">
          <p>{t("run.view.quiet", { duration: formatDuration(quietFor) })}</p>
        </div>
      )}
      {run.status === "failed" && run.errors.length === 0 && (
        <div class="notice bad" role="alert">
          <p>
            <strong>{t("run.view.runFailed")}</strong> {t("run.view.noReason")}
          </p>
        </div>
      )}
      {run.status === "cancelled" && run.errors.length === 0 && (
        <div class="notice warn" role="status">
          <p>{t("run.view.cancelledNotice")}</p>
        </div>
      )}

      {link.state !== "failed" && (
        <>
          <Timeline run={run} now={now} />
          {downloads.length > 0 && (
            <section class="panel">
              <h2>{t("run.view.texturedHeading")}</h2>
              <p class="sub">{t("run.view.texturedSub")}</p>
              {downloads.map((file) => (
                <button key={file.path} type="button" class="button primary" disabled={!canDownload(source) && !canShareFiles()}
                  onClick={() => void (canDownload(source) ? downloadFile(source, file.path) : shareFile(source, file.path)).catch((problem) => setDownloadError(describe(problem)))}>
                  {glbButton(canDownload(source) ? t("run.view.downloadGlb") : t("run.view.shareGlb"), file.stage)}
                </button>
              ))}
              {downloads[0] && <TexturePreview source={source} path={downloads[0].path} themeTick={themeTick} />}
              {source.kind === "local" && <p class="sub">{downloads.length > 1 ? t("run.view.savedInMore") : t("run.view.savedIn")}</p>}
              {downloadError && <p role="alert" class="field-error">{downloadError}</p>}
            </section>
          )}
          <div class="run-columns">
            <MeshPanel source={source} meshes={run.meshes} runStatus={run.status} themeTick={themeTick} />
            <ReportsPanel source={source} run={run} />
          </div>
          <InspectionPanel source={source} steps={run.inspections} />
          <Gallery source={source} run={run} />
        </>
      )}

      {source.replay !== undefined && link.state !== "failed" && <ReplayBar controls={source.replay} />}
    </section>
  );
}

function LinkNotice({ link }: { link: LinkStatus }) {
  if (link.state === "connecting") {
    return (
      <p class="sub" role="status">
        {t("run.view.loading")}
      </p>
    );
  }
  if (link.state === "retrying") {
    return (
      <div class="notice warn" role="status">
        <p>
          <strong>{t("run.view.connectionLost")}</strong> {t("run.view.tryingAgain", { message: link.message ?? "" })}
        </p>
      </div>
    );
  }
  if (link.state === "failed") {
    return (
      <div class="notice bad" role="alert">
        <p>
          <strong>{t("run.view.cannotShow")}</strong> {link.message}
        </p>
      </div>
    );
  }
  return null;
}

const STAGE_STATE_TEXT: Record<StageState["status"], MessageKey> = {
  pending: "run.stageState.pending",
  running: "run.stageState.running",
  done: "run.stageState.done",
  failed: "run.stageState.failed",
  cancelled: "run.stageState.cancelled",
  skipped: "run.stageState.skipped",
};

function Timeline({ run, now }: { run: RunState; now: number }) {
  return (
    <ol class="timeline" aria-label={t("run.view.stages")}>
      {run.stages.map((stage, index) => {
        const elapsed = stageElapsed(stage, now);
        const text = STAGE_TEXT[stage.name];
        return (
          <li
            key={stage.name}
            class={`stage stage-${stage.status}`}
            aria-current={stage.status === "running" ? "step" : undefined}
          >
            <div class="stage-top">
              <span class="stage-dot" aria-hidden="true">
                {stage.status === "done" ? <Icon name="check" size={14} /> : stage.status === "failed" ? "!" : index + 1}
              </span>
              <span class="stage-name">{stageTitle(stage.name)}</span>
              <span class="stage-state">
                {stage.status === "running" && stage.fraction > 0
                  ? formatPercent(stage.fraction, 0)
                  : t(STAGE_STATE_TEXT[stage.status])}
                {elapsed !== undefined && <span class="stage-time-inline"> · {formatDuration(elapsed)}</span>}
              </span>
            </div>
            <ProgressBar
              fraction={stage.status === "done" ? 1 : stage.fraction}
              label={t("run.progressLabel", { stage: stageTitle(stage.name) })}
              active={stage.status === "running"}
            />
            <div class="stage-bottom">
              <span class="stage-message">{stage.message !== "" ? stage.message : (text?.what ?? "")}</span>
              {elapsed !== undefined && <span class="stage-time">{formatDuration(elapsed)}</span>}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

function ReplayBar({ controls }: { controls: ReplayControls }) {
  const [state, setState] = useState<ReplaySnapshot>(() => controls.snapshot());
  const resumeAfterScrub = useRef(false);
  useEffect(() => {
    setState(controls.snapshot());
    return controls.subscribe(setState);
  }, [controls]);

  const speedLabel = (speed: number) => (speed === Infinity ? t("run.replay.instant") : t("run.replay.speed", { speed }));

  return (
    <div class="replay-bar" role="group" aria-label={t("run.replay.playback")}>
      <div class="replay-buttons">
        <button type="button" class="button icon-only" onClick={() => controls.step(-1)} disabled={state.position === 0} aria-label={t("run.replay.previous")}>
          <Icon name="prev" />
        </button>
        <button
          type="button"
          class="button primary icon-only"
          onClick={() => (state.playing ? controls.pause() : controls.play())}
          aria-label={state.playing ? t("run.replay.pause") : state.ended ? t("run.replay.again") : t("run.replay.play")}
          disabled={state.total === 0}
        >
          <Icon name={state.playing ? "pause" : state.ended ? "refresh" : "play"} />
        </button>
        <button type="button" class="button icon-only" onClick={() => controls.step(1)} disabled={state.ended} aria-label={t("run.replay.next")}>
          <Icon name="next" />
        </button>
      </div>
      <label class="replay-scrub">
        <span class="visually-hidden">{t("run.replay.position")}</span>
        <input
          type="range"
          min={0}
          max={state.total}
          step={1}
          value={state.position}
          aria-valuetext={t("run.replay.eventOf", { position: state.position, total: state.total })}
          onPointerDown={() => {
            resumeAfterScrub.current = state.playing;
            controls.pause();
          }}
          onPointerUp={() => {
            if (resumeAfterScrub.current && !controls.snapshot().ended) controls.play();
            resumeAfterScrub.current = false;
          }}
          onInput={(event) => controls.seek(Number(event.currentTarget.value))}
        />
      </label>
      <span class="replay-count" aria-hidden="true">
        {state.position} / {state.total}
      </span>
      <div class="segmented" role="group" aria-label={t("run.replay.speedGroup")}>
        {REPLAY_SPEEDS.map((speed) => (
          <button
            key={speed}
            type="button"
            aria-pressed={state.speed === speed}
            onClick={() => {
              controls.setSpeed(speed);
              savePrefs({ replaySpeed: speedToPref(speed) });
            }}
          >
            {speedLabel(speed)}
          </button>
        ))}
      </div>
    </div>
  );
}

/** Where the photos of this run come from, when the run says so (runs from downloaded example objects). */
function Attribution({ source }: { source: RunSource }) {
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    void source.attribution?.().then((value) => alive && setText(value));
    return () => {
      alive = false;
    };
  }, [source]);
  if (text === null) return null;
  return (
    <details class="raw attribution">
      <summary>{t("run.view.attribution")}</summary>
      <pre>{text}</pre>
    </details>
  );
}

/** A textured GLB's button: the run's mesh needs no name, the methods' surfaces do. */
function glbButton(action: string, stage: string | null | undefined): string {
  if (stage === "surface") return t("run.view.glbSurface", { action });
  if (stage === "splats") return t("run.view.glbSplats", { action });
  return action;
}
