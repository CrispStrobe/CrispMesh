import { useEffect, useState } from "preact/hooks";
import {
  sameConfig,
  sourceText,
  type ConfigField,
  type ToolCheck,
  type ToolName,
  type EngineStatus,
  type Shell,
  type ShellConfig,
  type ShellInfo,
  type ShellSettings,
} from "../shell/shell";
import { describe } from "../sources/transport";
import { t, tParts, type MessageKey } from "../i18n";

/** Polls the shell for the local engine's state: quickly while it starts, lazily once it runs. */
export function useLocalEngine(shell: Shell | null, wanted: boolean): { info: ShellInfo | null; status: EngineStatus | null; refresh(): void } {
  const [info, setInfo] = useState<ShellInfo | null>(null);
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (shell === null) return;
    shell.info().then(setInfo, () => undefined);
  }, [shell]);
  // The Python engine is only looked at, and only started, while it is the chosen engine.
  const canRun = info?.can_run_engine === true && wanted;
  const state = status?.state;
  useEffect(() => {
    if (shell !== null && canRun && state === "stopped") void shell.restartEngine().then(() => setTick((count) => count + 1));
  }, [shell, canRun, state]);
  useEffect(() => {
    if (shell === null || !canRun) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ask = () => {
      shell.engineStatus().then(
        (next) => {
          if (!alive) return;
          setStatus((previous) => (previous !== null && JSON.stringify(previous) === JSON.stringify(next) ? previous : next));
          timer = setTimeout(ask, next.state === "starting" ? 250 : 2500);
        },
        () => {
          if (alive) timer = setTimeout(ask, 2500);
        },
      );
    };
    ask();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
    // Re-arm when the state changes, so the polling pace follows it at once.
  }, [shell, canRun, state, tick]);
  return { info, status, refresh: () => setTick((count) => count + 1) };
}

/** Why Python is needed at all, next to its settings. */
export const notBundled = (): string => t("connection.shell.notBundled");

const STATE_TEXT: Record<EngineStatus["state"], MessageKey> = {
  stopped: "connection.shell.state.stopped",
  starting: "connection.shell.state.starting",
  running: "connection.shell.state.running",
  failed: "connection.shell.state.failed",
};

export function EngineLog({ status }: { status: EngineStatus }) {
  if (status.command === null && status.log.length === 0) return null;
  return (
    <details class="raw" open={status.state === "failed"}>
      <summary>{t("connection.shell.logSummary")}</summary>
      {status.command !== null && (
        <pre class="error-text" tabIndex={0}>
          {status.command}
        </pre>
      )}
      <pre class="error-text" tabIndex={0}>
        {status.log.length > 0 ? status.log.slice(-40).join("\n") : t("connection.shell.nothingPrinted")}
      </pre>
    </details>
  );
}

/** Shown instead of a screen that needs the local engine while it is not there. */
export function LocalEngineGate({ status, shell, onChange }: { status: EngineStatus | null; shell: Shell; onChange(): void }) {
  if (status === null || status.state === "starting" || status.state === "stopped") {
    return (
      <section class="page narrow">
        <p class="sub" role="status">
          {t("connection.shell.starting")}
        </p>
      </section>
    );
  }
  return (
    <section class="page narrow">
      <div class="notice bad" role="alert">
        <p>
          <strong>{t("connection.shell.couldNotStart")}</strong> {status.message}
        </p>
        <EngineLog status={status} />
        <div class="actions">
          <a class="button primary" href="#/shell">
            {t("connection.shell.engineSettings")}
          </a>
          <button
            type="button"
            class="button"
            onClick={() => {
              void shell.restartEngine().then(onChange);
            }}
          >
            {t("connection.shell.tryAgain")}
          </button>
          <a class="button" href="#/">
            {t("connection.shell.otherConnections")}
          </a>
        </div>
      </div>
      <p class="sub">{notBundled()}</p>
    </section>
  );
}

/** Texts are message keys, read when the screen renders (so they follow the language). */
interface FieldSpec {
  field: ConfigField;
  label: MessageKey;
  help: MessageKey;
  pick?: "folder" | "file";
  pickTitle?: MessageKey;
}

const PYTHON_FIELDS: FieldSpec[] = [
  {
    field: "repo",
    label: "connection.shell.field.repo",
    help: "connection.shell.field.repoHelp",
    pick: "folder",
    pickTitle: "connection.shell.field.repoPick",
  },
  {
    field: "python",
    label: "connection.shell.field.python",
    help: "connection.shell.field.pythonHelp",
    pick: "file",
    pickTitle: "connection.shell.field.pythonPick",
  },
  {
    field: "torch_python",
    label: "connection.shell.field.torchPython",
    help: "connection.shell.field.torchPythonHelp",
    pick: "file",
    pickTitle: "connection.shell.field.torchPythonPick",
  },
];

/** External programs of the photos start, grouped by the provider that needs them. */
const TOOL_GROUPS: { tool: ToolName; title: string; what: MessageKey; fields: FieldSpec[] }[] = [
  {
    tool: "alicevision",
    title: "AliceVision",
    what: "connection.shell.tool.alicevisionWhat",
    fields: [
      {
        field: "alicevision",
        label: "connection.shell.field.alicevision",
        help: "connection.shell.field.alicevisionHelp",
        pick: "folder",
        pickTitle: "connection.shell.field.alicevisionPick",
      },
      {
        field: "alicevision_library_path",
        label: "connection.shell.field.libraryPath",
        help: "connection.shell.field.libraryPathHelp",
      },
    ],
  },
  {
    tool: "colmap",
    title: "COLMAP",
    what: "connection.shell.tool.colmapWhat",
    fields: [
      {
        field: "colmap",
        label: "connection.shell.field.colmap",
        help: "connection.shell.field.colmapHelp",
        pick: "file",
        pickTitle: "connection.shell.field.colmapPick",
      },
    ],
  },
  {
    tool: "sam",
    title: "SAM 2.1",
    what: "connection.shell.tool.samWhat",
    fields: [
      { field: "sam_python", label: "connection.shell.field.torchPython", help: "connection.shell.field.samPythonHelp", pick: "file", pickTitle: "connection.shell.field.samPythonPick" },
      { field: "sam_source", label: "connection.shell.field.samSource", help: "connection.shell.field.samSourceHelp", pick: "folder", pickTitle: "connection.shell.field.samSourcePick" },
      { field: "sam_checkpoint", label: "connection.shell.field.samCheckpoint", help: "connection.shell.field.samCheckpointHelp", pick: "file", pickTitle: "connection.shell.field.samCheckpointPick" },
    ],
  },
];

const FOLDER_FIELDS: FieldSpec[] = [
  {
    field: "data_dir",
    label: "connection.shell.field.dataDir",
    help: "connection.shell.field.dataDirHelp",
    pick: "folder",
    pickTitle: "connection.shell.field.dataDirPick",
  },
  {
    field: "runs_dir",
    label: "connection.shell.field.runsDir",
    help: "connection.shell.field.runsDirHelp",
    pick: "folder",
    pickTitle: "connection.shell.field.runsDirPick",
  },
];

export function ShellSettingsScreen({
  shell,
  info,
  status,
  mode,
  onChange,
}: {
  shell: Shell;
  info: ShellInfo | null;
  status: EngineStatus | null;
  mode: "native" | "python" | "remote" | "browser";
  onChange(): void;
}) {
  const [settings, setSettings] = useState<ShellSettings | null>(null);
  const [form, setForm] = useState<ShellConfig | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [checks, setChecks] = useState<Partial<Record<ToolName, ToolCheck | "busy">>>({});

  useEffect(() => {
    shell.settings().then(
      (loaded) => {
        setSettings(loaded);
        setForm(loaded.saved);
      },
      (problem) => setError(describe(problem)),
    );
  }, [shell]);

  if (error !== "" && settings === null) {
    return (
      <section class="page narrow">
        <div class="notice bad" role="alert">
          <p>
            <strong>{t("connection.shell.readFailed")}</strong> {error}
          </p>
        </div>
      </section>
    );
  }
  if (settings === null || form === null) return null;
  const python = info?.can_run_engine === true;
  const sandboxed = info?.sandboxed === true;

  const change = (field: ConfigField, value: string) => setForm({ ...form, [field]: value });
  const dirty = !sameConfig(form, settings.saved);
  const resolvedOf = (field: ConfigField) =>
    (settings.tools as Record<string, { value: string; source: "setting" | "environment" | "found" | "default" }>)[field] ??
    (settings.resolved as Record<string, { value: string; source: "setting" | "environment" | "found" | "default" }>)[field]!;

  const apply = async (event: Event) => {
    event.preventDefault();
    setBusy(true);
    setError("");
    try {
      const saved = await shell.saveSettings(form);
      setSettings(saved);
      setForm(saved.saved);
      if (mode === "python") await shell.restartEngine();
      onChange();
    } catch (problem) {
      setError(describe(problem));
    } finally {
      setBusy(false);
    }
  };

  // A check uses what is saved, so unsaved changes are saved first.
  const check = async (tool: ToolName) => {
    setChecks((current) => ({ ...current, [tool]: "busy" }));
    try {
      if (!sameConfig(form, settings.saved)) {
        const saved = await shell.saveSettings(form);
        setSettings(saved);
        setForm(saved.saved);
      }
      const result = await shell.checkTool(tool);
      setChecks((current) => ({ ...current, [tool]: result }));
    } catch (problem) {
      setChecks((current) => ({ ...current, [tool]: { ok: false, summary: describe(problem), detail: "" } }));
    }
  };

  const pick = async (spec: FieldSpec) => {
    if (spec.pick === undefined) return;
    const current = form[spec.field] || resolvedOf(spec.field).value;
    const chosen = await shell.pickPath(spec.pick, t(spec.pickTitle ?? spec.label), current || undefined).catch(() => null);
    if (chosen !== null) change(spec.field, chosen);
  };

  return (
    <section class="page narrow">
      <h1>{t("connection.shell.title")}</h1>
      {info?.native_engine && (
        <p class="sub">
          {t(mode === "native" ? "connection.shell.builtInInUse" : "connection.shell.builtIn")} <a href="#/engine">{t("connection.shell.openRuns")}</a>
        </p>
      )}

      {python && mode === "python" && (
      <section class="panel" aria-labelledby="shell-state">
        <div class="panel-head">
          <h2 id="shell-state">{t("connection.shell.python")}</h2>
          <span class={`badge status-${status?.state === "running" ? "complete" : status?.state === "failed" ? "failed" : "running"}`} role="status">
            {status === null ? t("connection.shell.state.unknown") : t(STATE_TEXT[status.state])}
          </span>
        </div>
        {status?.state === "running" && (
          <p class="sub">
            {tParts("connection.shell.listening", { url: <span class="mono">{status.url}</span>, device: status.device ?? "" })}{" "}
            <a href="#/engine">{t("connection.shell.openRuns")}</a>
          </p>
        )}
        {status?.state === "failed" && (
          <p class="field-error" role="alert">
            {status.message}
          </p>
        )}
        {status !== null && <EngineLog status={status} />}
      </section>
      )}

      {sandboxed && (
        <div class="notice" role="note">
          <p>{t("connection.shell.sandboxed")}</p>
        </div>
      )}
      <form onSubmit={apply} novalidate>
        {[
          { legend: t("connection.shell.folders"), fields: FOLDER_FIELDS, shown: true },
          ...TOOL_GROUPS.map((group) => ({
            legend: t("connection.shell.tools", { name: group.title }),
            fields: group.fields,
            shown: info?.native_engine === true && !sandboxed,
            tool: group.tool,
            what: group.what,
          })),
          { legend: t("connection.shell.pythonOptional"), fields: PYTHON_FIELDS, shown: python },
        ]
          .filter((group) => group.shown)
          .map((group) => (
        <fieldset class="panel" key={group.legend}>
          <legend>{group.legend}</legend>
          {"tool" in group && (
            <>
              <p class="help">{t(group.what)}</p>
            </>
          )}
          {group.fields === PYTHON_FIELDS && (
            <p class="help">
              <strong>{t("connection.shell.pythonNotPart")}</strong> {notBundled()}
            </p>
          )}
          {group.fields.map((spec) => {
            const resolved = resolvedOf(spec.field);
            const id = `shell-${spec.field}`;
            return (
              <div class="field" key={spec.field}>
                <label class="field-label" for={id}>
                  {t(spec.label)}
                </label>
                <div class="path-row">
                  <input
                    id={id}
                    type="text"
                    spellcheck={false}
                    autocapitalize="off"
                    autocomplete="off"
                    value={form[spec.field]}
                    placeholder={resolved.source === "setting" ? "" : resolved.value || t("connection.shell.notFound")}
                    aria-describedby={`${id}-help`}
                    onInput={(event) => change(spec.field, event.currentTarget.value)}
                  />
                  {spec.pick !== undefined && (
                    <button type="button" class="button" onClick={() => void pick(spec)}>
                      {t("connection.shell.choose")}<span class="visually-hidden"> {t(spec.label)}</span>
                    </button>
                  )}
                </div>
                <span class="help" id={`${id}-help`}>
                  {t(spec.help)}
                  {form[spec.field].trim() === "" && (
                    <>
                      {" "}
                      {resolved.value === ""
                        ? t("connection.shell.notSetNothing")
                        : t("connection.shell.notSetUsed", { source: sourceText(resolved.source, spec.field) })}
                    </>
                  )}
                </span>
              </div>
            );
          })}
          {group.fields === PYTHON_FIELDS && (
          <label class="field" for="shell-device">
            <span class="field-label">{t("connection.shell.device")}</span>
            <select id="shell-device" value={form.device === "" ? "auto" : form.device} onChange={(event) => change("device", event.currentTarget.value)}>
              <option value="auto">{info !== null ? t("connection.shell.automaticOn", { device: info.auto_device }) : t("connection.shell.automatic")}</option>
              <option value="mps">Apple GPU (mps)</option>
              <option value="cuda">NVIDIA GPU (cuda)</option>
              <option value="cpu">CPU</option>
            </select>
            <span class="help">{t("connection.shell.deviceHelp")}</span>
          </label>
          )}
          {"tool" in group && (
            <div class="tool-check">
              <button type="button" class="button" disabled={checks[group.tool] === "busy"} onClick={() => void check(group.tool)}>
                {checks[group.tool] === "busy" ? t("connection.shell.checking") : t("connection.shell.check")}
                <span class="visually-hidden"> {group.legend}</span>
              </button>
              {(() => {
                const result = checks[group.tool];
                if (result === undefined || result === "busy") return null;
                return (
                  <div class={`tool-result ${result.ok ? "ok" : "bad"}`} role="status">
                    <strong>{result.ok ? t("connection.shell.works") : t("connection.shell.doesNotWork")}</strong> {result.summary}
                    {result.detail !== "" && (
                      <details class="raw">
                        <summary>{t("connection.shell.whatWasRun")}</summary>
                        <pre class="error-text" tabIndex={0}>
                          {result.detail}
                        </pre>
                      </details>
                    )}
                  </div>
                );
              })()}
            </div>
          )}
        </fieldset>
          ))}

        {error !== "" && (
          <div class="notice bad" role="alert">
            <p>{error}</p>
          </div>
        )}
        <div class="actions">
          <button type="submit" class="button primary" disabled={busy}>
            {mode === "python" ? (dirty ? t("connection.shell.saveRestart") : t("connection.shell.restart")) : t("connection.shell.save")}
          </button>
          <a class="button" href="#/engine">
            {t("connection.shell.runs")}
          </a>
        </div>
        <p class="help">
          {tParts("connection.shell.savedIn", { file: <span class="mono wrap">{settings.file}</span> })}
        </p>
      </form>
    </section>
  );
}
