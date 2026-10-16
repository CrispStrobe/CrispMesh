import { useEffect, useRef, useState } from "preact/hooks";
import { formatDuration, formatPercent } from "../core/format";
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
            <Icon name="back" size={16} /> {source.kind === "replay" ? "Connection" : "Runs"}
          </a>
          <h1>{heading}</h1>
          <p class="sub run-meta">
            <StatusBadge status={run.status} />
            {total !== undefined && <span>{formatDuration(total)}</span>}
            {run.device !== undefined && <span>on {run.device}</span>}
            {source.kind === "replay" && <span class="tag">Recording</span>}
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
            {cancel.state === "idle" ? "Cancel run" : cancel.state === "asking" ? "Cancelling..." : "Stopping..."}
          </button>
        )}
      </div>

      <LinkNotice link={link} />
      {cancel.error !== undefined && (
        <div class="notice bad" role="alert" id="cancel-error">
          <p>
            <strong>The run could not be cancelled.</strong> {cancel.error}
          </p>
        </div>
      )}
      {run.schemaUnknown && (
        <div class="notice warn" role="status">
          <p>
            This run uses a newer event format ({run.schema}). Studio shows the parts it understands.
          </p>
        </div>
      )}
      {run.errors.map((error) =>
        run.status === "cancelled" ? (
          <div class="notice warn" role="status" key={error.seq}>
            <p>
              <strong>The run was cancelled</strong>
              {error.stage !== null && <> during {stageTitle(error.stage).toLowerCase()}</>}.
            </p>
            <details class="raw">
              <summary>Last output of the stage</summary>
              <pre class="error-text" tabIndex={0}>
                {error.message}
              </pre>
            </details>
          </div>
        ) : (
          <div class="notice bad" role="alert" key={error.seq}>
            <p>
              <strong>{error.stage !== null ? `${stageTitle(error.stage)} failed` : "The run failed"}.</strong>
            </p>
            <pre class="error-text" tabIndex={0}>
              {error.message}
            </pre>
          </div>
        ),
      )}
      {quietFor !== undefined && quietFor > QUIET_AFTER_SECONDS && (
        <div class="notice" role="status">
          <p>
            No news from the engine for {formatDuration(quietFor)}. Long steps can be quiet. If this goes on, the run may
            have stopped without saying so; the engine's log will tell.
          </p>
        </div>
      )}
      {run.status === "failed" && run.errors.length === 0 && (
        <div class="notice bad" role="alert">
          <p>
            <strong>The run failed.</strong> The engine gave no reason.
          </p>
        </div>
      )}
      {run.status === "cancelled" && run.errors.length === 0 && (
        <div class="notice warn" role="status">
          <p>The run was cancelled.</p>
        </div>
      )}

      {link.state !== "failed" && (
        <>
          <Timeline run={run} now={now} />
          {run.downloads.length > 0 && (
            <section class="panel">
              <h2>Textured model</h2>
              <p class="sub">GLB includes the photo texture and the same reconstructed geometry.</p>
              {run.downloads.map((file) => (
                <button key={file.path} type="button" class="button primary" disabled={!canDownload(source) && !canShareFiles()}
                  onClick={() => void (canDownload(source) ? downloadFile(source, file.path) : shareFile(source, file.path)).catch((problem) => setDownloadError(describe(problem)))}>
                  {(canDownload(source) ? "Download textured GLB" : "Share textured GLB") + glbOf(file.stage)}
                </button>
              ))}
              {run.downloads[0] && <TexturePreview source={source} path={run.downloads[0].path} themeTick={themeTick} />}
              {source.kind === "local" && <p class="sub">Saved in this run’s folder as texture/mesh.glb{run.downloads.length > 1 ? " (the other surfaces’ in surface/texture and splats/texture)" : ""}.</p>}
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
        Loading...
      </p>
    );
  }
  if (link.state === "retrying") {
    return (
      <div class="notice warn" role="status">
        <p>
          <strong>Connection lost.</strong> {link.message} Trying again.
        </p>
      </div>
    );
  }
  if (link.state === "failed") {
    return (
      <div class="notice bad" role="alert">
        <p>
          <strong>This run cannot be shown.</strong> {link.message}
        </p>
      </div>
    );
  }
  return null;
}

const STAGE_STATE_TEXT: Record<StageState["status"], string> = {
  pending: "Waiting",
  running: "Running",
  done: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
  skipped: "Not run",
};

function Timeline({ run, now }: { run: RunState; now: number }) {
  return (
    <ol class="timeline" aria-label="Stages">
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
                  : STAGE_STATE_TEXT[stage.status]}
                {elapsed !== undefined && <span class="stage-time-inline"> · {formatDuration(elapsed)}</span>}
              </span>
            </div>
            <ProgressBar
              fraction={stage.status === "done" ? 1 : stage.fraction}
              label={`${stageTitle(stage.name)} progress`}
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

  const speedLabel = (speed: number) => (speed === Infinity ? "Instant" : `${speed}×`);

  return (
    <div class="replay-bar" role="group" aria-label="Playback">
      <div class="replay-buttons">
        <button type="button" class="button icon-only" onClick={() => controls.step(-1)} disabled={state.position === 0} aria-label="Previous event">
          <Icon name="prev" />
        </button>
        <button
          type="button"
          class="button primary icon-only"
          onClick={() => (state.playing ? controls.pause() : controls.play())}
          aria-label={state.playing ? "Pause" : state.ended ? "Play again from the start" : "Play"}
          disabled={state.total === 0}
        >
          <Icon name={state.playing ? "pause" : state.ended ? "refresh" : "play"} />
        </button>
        <button type="button" class="button icon-only" onClick={() => controls.step(1)} disabled={state.ended} aria-label="Next event">
          <Icon name="next" />
        </button>
      </div>
      <label class="replay-scrub">
        <span class="visually-hidden">Position in the recording, by event</span>
        <input
          type="range"
          min={0}
          max={state.total}
          step={1}
          value={state.position}
          aria-valuetext={`Event ${state.position} of ${state.total}`}
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
      <div class="segmented" role="group" aria-label="Playback speed">
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
      <summary>Source and license of the photos</summary>
      <pre>{text}</pre>
    </details>
  );
}

/** Which surface a textured GLB belongs to, for its button: the run's mesh needs no name, the methods' do. */
function glbOf(stage: string | null | undefined): string {
  if (stage === "surface") return " (neural surface)";
  if (stage === "splats") return " (2D splats)";
  return "";
}
