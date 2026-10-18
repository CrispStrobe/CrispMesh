import { engineText } from "../i18n";
import { t } from "../i18n";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { fieldOfViewOf } from "../core/captureLens";
import { fewPhotos, surfaceAutomatically } from "../core/fewPhotos";
import { humanise } from "../core/format";
import {
  defaultFormValue,
  defaultFormValues,
  diffSettings,
  groupSettings,
  isChanged,
  placeServerError,
  type FormValues,
  type SettingSpec,
} from "../core/settings";
import type { OptionValues } from "../core/providerOptions";
import { activeOptions, CONTRACT_START_POINTS, importFields, missingFields, startBody, startOptionErrors } from "../core/startPoints";
import { describe, isAbort } from "../sources/transport";
import { EngineError, type Engine, type EngineHealth } from "../sources/types";
import { navigate } from "./app";
import type { BrowserEngine } from "../sources/browserEngine";
import { DeviceImport } from "./deviceImport";
import { FolderPicker } from "./folderPicker";
import { CalibrationPicker, PhotosPicker } from "./photoPickers";
import { CaptureButton } from "./capture";
import { Icon } from "./icons";
import { PathField } from "./pathField";
import { CalibrationList, ProviderChoices } from "./providerFields";
import type { Prefs } from "./prefs";

interface Props {
  engine: Engine;
  prefs: Prefs;
  onChange(change: Partial<Prefs>): void;
}

const DEVICES = [
  { value: "", label: "newRun.engineDefault" },
  { value: "mps", label: "newRun.appleGpu" },
  { value: "cuda", label: "newRun.nvidiaGpu" },
  { value: "cpu", label: "newRun.cpu" },
] as const;

export function NewRun({ engine, prefs, onChange }: Props) {
  const [specs, setSpecs] = useState<SettingSpec[] | null>(null);
  const [health, setHealth] = useState<EngineHealth | null>(null);
  const [loadError, setLoadError] = useState("");
  const [form, setForm] = useState<FormValues>({});
  const [texture, setTexture] = useState(false);
  // Optional photo-optimised surfaces: quality level per method ("" = off) and single numeric settings.
  const [methodQuality, setMethodQuality] = useState<Record<string, string>>({});
  const [methodSettings, setMethodSettings] = useState<Record<string, Record<string, string>>>({});
  // Number of photos chosen, where the form knows it (picked here, taken with the camera, copied in); null for a typed path.
  const [photoCount, setPhotoCount] = useState<number | null>(() =>
    engine.kind === "browser" ? ((engine as BrowserEngine).pickedPhotos()?.files.length ?? null) : null,
  );
  // The neural surface was switched on because of few photos (once per form; `surfaceDecided` keeps it from happening again).
  const [surfaceSwitched, setSurfaceSwitched] = useState(false);
  const surfaceDecided = useRef(false);
  const captureUsedForeground = useRef(false);
  const [name, setName] = useState("");
  const [values, setValues] = useState<Record<string, string>>({ inputs: engine.kind === "browser" ? ((engine as BrowserEngine).inputs()?.name ?? "") : prefs.inputs });
  const [pointId, setPointId] = useState("inputs");
  const [providers, setProviders] = useState<Record<string, string>>({});
  const [providerOptions, setProviderOptions] = useState<OptionValues>({});
  const [reference, setReference] = useState("");
  const [device, setDevice] = useState(prefs.device);
  const [busy, setBusy] = useState(false);
  const [serverError, setServerError] = useState<{ fields: string[]; message: string } | null>(null);
  const [showErrors, setShowErrors] = useState(false);

  useEffect(() => {
    const abort = new AbortController();
    engine
      .settings(abort.signal)
      .then((loaded) => {
        setSpecs(loaded);
        setForm(defaultFormValues(loaded));
      })
      .catch((problem) => {
        if (!isAbort(problem)) setLoadError(describe(problem));
      });
    engine
      .health(abort.signal)
      .then(setHealth)
      .catch(() => undefined);
    return () => abort.abort();
  }, [engine]);

  const groups = useMemo(() => groupSettings(specs ?? []), [specs]);
  const diff = useMemo(() => diffSettings(specs ?? [], form), [specs, form]);
  const changedCount = Object.keys(diff.changed).length + Object.keys(diff.errors).length;
  const points = health?.startPoints ?? CONTRACT_START_POINTS;
  const point = points.find((candidate) => candidate.id === pointId) ?? points[0]!;
  const missing = missingFields(point, values, providers);
  const optionProblems = point.providers.length > 0 ? startOptionErrors(point, providers, providerOptions) : {};
  const methods = (health?.methods ?? []).filter((method) => method.available);
  // The engines built from this crate texture meshes: this device's, the browser's, and another computer's app.
  const texturing = engine.kind === "browser" || engine.kind === "local" || health?.methods !== undefined;
  const methodErrors: Record<string, string> = {};
  for (const method of methods) {
    if ((methodQuality[method.id] ?? "") === "") continue;
    for (const [settingName, value] of Object.entries(methodSettings[method.id] ?? {})) {
      if (value.trim() !== "" && !Number.isFinite(Number(value))) methodErrors[`${method.id}.${settingName}`] = t("newRun.number");
    }
  }
  const methodBody = (): Record<string, unknown> => {
    const body: Record<string, unknown> = {};
    for (const method of methods) {
      const quality = methodQuality[method.id] ?? "";
      if (quality === "") continue;
      const settings = Object.entries(methodSettings[method.id] ?? {})
        .filter(([, value]) => value.trim() !== "")
        .map(([settingName, value]) => `${settingName}=${value.trim()}`);
      body[method.id] = { quality, settings };
    }
    return body;
  };
  // Fewer than 25 photos per turn: the ordinary mesh gets rough, the neural surface (fast) copes with about 20.
  const few = point.fields.some((field) => field.key === "photos") && fewPhotos(photoCount, providerOptions["turntable-rings"], providerOptions["turntable-turn-sizes"]);
  const surfaceOffered = methods.some((method) => method.id === "surface" && method.qualities.some((level) => level.id === "fast"));
  useEffect(() => {
    const choice = surfaceAutomatically({ few, offered: surfaceOffered, quality: methodQuality.surface ?? "", decided: surfaceDecided.current });
    surfaceDecided.current = choice.decided;
    if (!choice.switchOn) return;
    setMethodQuality((current) => ({ ...current, surface: "fast" }));
    setSurfaceSwitched(true);
  }, [few, surfaceOffered]);
  const choosesDevice = health?.choosesDevice !== false;
  const scoresReference = health?.scoresReference !== false;
  const capturedTurns = (sizes: number[]) => setProviderOptions((current) => ({
    ...current, "turntable-rings": "1", "turntable-turn-sizes": sizes.length ? sizes.join(",") : "",
  }));
  const clearCapturedPhotos = () => {
    capturedTurns([]);
    if (captureUsedForeground.current) {
      setProviders(current => ({ ...current, masks: "threshold" }));
      setValues(current => ({ ...current, masks_import: "" }));
      setProviderOptions(current => ({ ...current, "object-crop-padding": "-1" }));
      captureUsedForeground.current = false;
    }
  };
  const setField = (key: string, value: string) => {
    setValues((current) => ({ ...current, [key]: value }));
    setServerError(null);
  };

  const setValue = (settingName: string, value: string | boolean) => {
    setForm((current) => ({ ...current, [settingName]: value }));
    setServerError(null);
  };

  const submit = async (event: Event) => {
    event.preventDefault();
    setShowErrors(true);
    setServerError(null);
    if (missing.length > 0 || Object.keys(optionProblems).length > 0 || Object.keys(diff.errors).length > 0 || Object.keys(methodErrors).length > 0) {
      // Open the groups that hold a problem and move to the first one.
      requestAnimationFrame(() => {
        const first = document.querySelector<HTMLElement>(".new-run [aria-invalid='true']");
        first?.closest("details")?.setAttribute("open", "");
        first?.focus();
      });
      return;
    }
    setBusy(true);
    try {
      const supported = new Set(activeOptions(point, providers).map(option => option.name));
      const turns = String(providerOptions["turntable-turn-sizes"] ?? "").split(",").filter(Boolean);
      if ((turns.length > 1 && !supported.has("turntable-turn-sizes")) ||
          (captureUsedForeground.current && !supported.has("object-crop-padding"))) {
        throw new Error(t("newRun.updateCaptureEngine"));
      }
      const start = { name, device: choosesDevice ? device : "", reference: scoresReference ? reference : "", values, providers, options: providerOptions };
      const id = await engine.startRun({ ...startBody(point, start, diff.changed), ...(texturing ? { texture } : {}), ...methodBody() });
      if (engine.kind !== "browser") onChange({ inputs: (values.inputs ?? "").trim(), device });
      navigate(`#/engine/run/${encodeURIComponent(id)}`);
    } catch (problem) {
      const message = describe(problem);
      const names = [...(specs ?? []).map((spec) => spec.name), ...point.fields.map((field) => field.key), ...importFields(point, providers).map((entry) => entry.spec.key), "reference", "device", "name"];
      setServerError(
        problem instanceof EngineError && problem.status === 400 ? placeServerError(message, names) : { fields: [], message },
      );
      requestAnimationFrame(() => document.getElementById("server-error")?.focus());
    } finally {
      setBusy(false);
    }
  };

  const fieldError = (field: string): string | undefined => {
    if (serverError?.fields.includes(field)) return serverError.message;
    return showErrors ? diff.errors[field] : undefined;
  };

  if (loadError !== "") {
    return (
      <section class="page narrow">
        <div class="notice bad" role="alert">
          <p>
            <strong>{t("newRun.loadFailed")}</strong> {loadError}
          </p>
          <a class="button" href="#/engine">
            {t("newRun.back")}
          </a>
        </div>
      </section>
    );
  }


  return (
    <section class="page narrow new-run">
      <a class="back-link" href="#/engine">
        <Icon name="back" size={16} /> {t("newRun.runs")}
      </a>
      <h1>{t("newRun.title")}</h1>
      {health?.note !== undefined && <p class="notice">{health.note}</p>}
      <p class="sub" hidden={engine.kind === "browser"}>
        {t("newRun.intro")}
        {engine.pickPath !== undefined
          ? t("newRun.pathBrowse")
          : t("newRun.pathRelative")}
        {health?.dataFolder !== undefined && (
          <>
            {" "}
            {t("newRun.dataFolder")} <span class="mono wrap">{health.dataFolder}</span>
          </>
        )}
      </p>

      <form onSubmit={submit} novalidate>
        <fieldset class="panel">
          <legend>{t("newRun.data")}</legend>
          {points.length > 1 && (
            <div class="field">
              <span class="field-label" id="f-start-label">
                {t("newRun.startFrom")}
              </span>
              <div class="segmented wrap-segments" role="group" aria-labelledby="f-start-label">
                {points.map((candidate) => (
                  <button
                    key={candidate.id}
                    type="button"
                    aria-pressed={candidate.id === point.id}
                    onClick={() => {
                      setPointId(candidate.id);
                      setServerError(null);
                    }}
                  >
                    {engineText(candidate.label)}
                  </button>
                ))}
              </div>
              {point.meaning !== undefined && <span class="help">{engineText(point.meaning)}</span>}
            </div>
          )}
          {point.fields.map((field) =>
            engine.kind === "browser" && field.kind === "inputs" ? (
              <FolderPicker
                key={`${point.id}-${field.key}`}
                engine={engine as BrowserEngine}
                error={fieldError(field.key) ?? (showErrors && missing.includes(field.key) ? t("newRun.chooseInputs") : undefined)}
                onPicked={(name) => setField(field.key, name)}
              />
            ) : engine.kind === "browser" && field.key === "photos" ? (
              <PhotosPicker
                key={`${point.id}-${field.key}`}
                engine={engine as BrowserEngine}
                error={fieldError(field.key) ?? (showErrors && missing.includes(field.key) ? t("newRun.choosePhotos") : undefined)}
                onPicked={(name) => setField(field.key, name)}
                onAutomaticLens={(label) => setField("calibration", label)}
                onCount={setPhotoCount}
                onTurns={capturedTurns}
              />
            ) : engine.kind === "browser" && field.key === "calibration" ? (
              <CalibrationPicker
                key={`${point.id}-${field.key}`}
                engine={engine as BrowserEngine}
                value={values[field.key]}
                error={fieldError(field.key) ?? (showErrors && missing.includes(field.key) ? t("newRun.chooseLens") : undefined)}
                onPicked={(name) => setField(field.key, name)}
              />
            ) : (
            <PathField
              key={`${point.id}-${field.key}`}
              engine={engine}
              want={field.kind}
              id={`f-${field.key}`}
              label={engineText(field.label)}
              optional={field.required ? undefined : t("newRun.optional")}
              value={values[field.key] ?? ""}
              onInput={(value) => {
                setField(field.key, value);
                // A typed or browsed folder: its number of photos is not known here.
                if (field.key === "photos") { setPhotoCount(null); clearCapturedPhotos(); }
              }}
              placeholder={field.kind === "inputs" ? "dragon/inputs" : undefined}
              help={engineText(field.help)}
              error={
                fieldError(field.key) ??
                (showErrors && missing.includes(field.key)
                  ? field.kind === "inputs"
                    ? t("newRun.whereInputs")
                    : t("newRun.whereField", { field: engineText(field.label) })
                  : undefined)
              }
              required={field.required}
            >
              {health?.imports === true && (field.key === "photos" || field.key === "calibration") && (
                <DeviceImport
                  engine={engine}
                  what={field.key}
                  onImported={(path, count) => {
                    setField(field.key, path);
                    if (field.key === "photos") { setPhotoCount(count); clearCapturedPhotos(); }
                  }}
                />
              )}
              {field.key === "photos" && point.fields.some((other) => other.automatic === true) && (
                <CaptureButton
                  engine={engine}
                  onSaved={(folder, count, calibration, sizes, foreground) => {
                    setField("photos", folder);
                    setPhotoCount(count);
                    capturedTurns(sizes);
                    if (foreground) {
                      setProviders(current=>({...current,masks:"import"}));
                      setField("masks_import",foreground.folder);
                      setProviderOptions(current=>({...current,"object-crop-padding":String(foreground.padding)}));
                    } else if (captureUsedForeground.current) {
                      setProviders(current=>({...current,masks:"threshold"}));
                      setField("masks_import","");
                      setProviderOptions(current=>({...current,"object-crop-padding":"-1"}));
                    }
                    captureUsedForeground.current = foreground !== undefined;
                    // Photos from this device's camera have no lens file: the camera's field of view where the app
                    // knows it (fov:DEG), else the focal length is found from the turn (auto). A file chosen is kept.
                    const current = (values.calibration ?? "").trim();
                    if (current === "" || current === "auto" || fieldOfViewOf(current) !== null) setField("calibration", calibration);
                  }}
                />
              )}
              {field.key === "calibration" && <CalibrationList engine={engine} current={values.calibration ?? ""} onPick={(path) => setField("calibration", path)} />}
              {field.automatic === true && (
                <div class="calibrations" role="group" aria-label={t("newRun.noLens")}>
                  <span class="help">{t("newRun.noFile")}</span>
                  <button type="button" class="button small" aria-pressed={values[field.key] === "auto"} onClick={() => setField(field.key, "auto")}>
                    {t("newRun.automatic")}
                  </button>
                  <span class="help">{t("newRun.lensHelp")}</span>
                  {fieldOfViewOf(values[field.key] ?? "") !== null && (
                    <span class="help" data-lens="field-of-view">
                      {t("newRun.fieldOfView", { degrees: fieldOfViewOf(values[field.key] ?? "")!.toFixed(1) })}
                    </span>
                  )}
                </div>
              )}
            </PathField>
            ),
          )}
          {point.providers.length > 0 && (
            <ProviderChoices
              engine={engine}
              point={point}
              providers={providers}
              onProvider={(module, id) => {
                setProviders((current) => ({ ...current, [module]: id }));
                setServerError(null);
              }}
              options={providerOptions}
              onOption={(flag, value) => {
                setProviderOptions((current) => ({ ...current, [flag]: value }));
                setServerError(null);
              }}
              values={values}
              onValue={setField}
              error={(key) =>
                fieldError(key) ?? (showErrors && missing.includes(key) ? t("newRun.where") : showErrors ? optionProblems[key] : undefined)
              }
              toolsHref={engine.pickPath !== undefined ? "#/shell" : undefined}
            />
          )}
          {scoresReference && (
            <PathField
              engine={engine}
              want="file"
              id="f-reference"
              label={t("newRun.reference")}
              optional={t("newRun.optional")}
              value={reference}
              onInput={(value) => {
                setReference(value);
                setServerError(null);
              }}
              placeholder="dragon/scan.ply"
              help={t("newRun.referenceHelp")}
              error={fieldError("reference")}
            />
          )}
          <div class="field-row">
            <TextField
              id="f-name"
              label={t("newRun.name")}
              optional={t("newRun.optional")}
              value={name}
              onInput={setName}
              placeholder={t("newRun.myDragon")}
              error={fieldError("name")}
            />
            {choosesDevice && (
              <label class="field" for="f-device">
                <span class="field-label">{t("newRun.device")}</span>
                <select id="f-device" value={device} onChange={(event) => setDevice(event.currentTarget.value)}>
                  {DEVICES.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.value === "" && health?.device !== undefined ? t("newRun.deviceDefault", { device: health.device }) : t(option.label)}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
        </fieldset>

        {texturing && (
          <label class="field">
            <span><input type="checkbox" checked={texture} onChange={(event) => setTexture(event.currentTarget.checked)} /> {t("newRun.texture")}</span>
            <span class="sub">{t("newRun.textureHelp")}</span>
          </label>
        )}

        {few && !surfaceOffered && (
          <p class="notice warn" role="status">
            {t("newRun.few")}
          </p>
        )}

        {methods.length > 0 && (
          <fieldset class="methods">
            <legend>{t("newRun.surfaces")}</legend>
            <p class="sub">{t("newRun.surfacesHelp")}</p>
            {few && surfaceOffered && (
              <p class="notice warn" role="status">
                {t("newRun.fewSurface")}
                {surfaceSwitched && methodQuality.surface === "fast" && t("newRun.autoSwitched")}
              </p>
            )}
            {methods.map((method) => {
              const quality = methodQuality[method.id] ?? "";
              const chosen = method.qualities.find((level) => level.id === quality);
              return (
                <div class="field" key={method.id}>
                  <label class="field-label" for={`f-method-${method.id}`}>{engineText(method.label)}</label>
                  <select
                    id={`f-method-${method.id}`}
                    value={quality}
                    onChange={(event) => setMethodQuality((current) => ({ ...current, [method.id]: event.currentTarget.value }))}
                  >
                    <option value="">{t("newRun.offLabel")}</option>
                    {method.qualities.map((level) => (
                      <option key={level.id} value={level.id}>{engineText(level.label)}</option>
                    ))}
                  </select>
                  <span class="sub">{chosen ? chosen.meaning : method.meaning}</span>
                  {quality !== "" && method.settings.length > 0 && (
                    <details>
                      <summary>{t("newRun.methodSettings", { method: engineText(method.label) })}</summary>
                      {method.settings.map((setting) => {
                        const key = `${method.id}.${engineText(setting.name)}`;
                        const error = showErrors ? methodErrors[key] : undefined;
                        return (
                          <label class="field" key={key}>
                            <span class="field-label">{engineText(setting.name)}</span>
                            <input
                              inputMode="numeric"
                              placeholder={t("newRun.levelValue")}
                              aria-invalid={error !== undefined}
                              value={methodSettings[method.id]?.[setting.name] ?? ""}
                              onInput={(event) => {
                                const value = event.currentTarget.value;
                                setMethodSettings((current) => ({ ...current, [method.id]: { ...(current[method.id] ?? {}), [setting.name]: value } }));
                              }}
                            />
                            <span class="sub">{engineText(setting.meaning)}</span>
                            {error && <span class="field-error">{error}</span>}
                          </label>
                        );
                      })}
                    </details>
                  )}
                </div>
              );
            })}
          </fieldset>
        )}

        <div class="settings-head">
          <h2>{t("newRun.settings")}</h2>
          <span class="sub" aria-live="polite">
            {changedCount === 0 ? t("newRun.defaults") : t("newRun.changed", { count: changedCount })}
          </span>
          <button
            type="button"
            class="button"
            disabled={changedCount === 0}
            onClick={() => {
              setForm(defaultFormValues(specs ?? []));
              setServerError(null);
            }}
          >
            {t("newRun.reset")}
          </button>
        </div>

        {specs === null && <p class="sub">{t("newRun.loading")}</p>}

        {groups.map((group, index) => {
          const changedHere = group.settings.filter((spec) => isChanged(spec, form[spec.name])).length;
          return (
            <details class="panel settings-group" key={engineText(group.group)} open={index === 0}>
              <summary>
                <span class="group-name">{engineText(group.group)}</span>
                <span class="sub">
                  {t("newRun.settingsCount", { count: group.settings.length })}{changedHere > 0 ? t("newRun.changedHere", { count: changedHere }) : ""}
                </span>
              </summary>
              <div class="settings-grid">
                {group.settings.map((spec) => (
                  <SettingField
                    key={spec.name}
                    spec={spec}
                    value={form[spec.name] ?? defaultFormValue(spec)}
                    error={fieldError(spec.name)}
                    onChange={(value) => setValue(spec.name, value)}
                  />
                ))}
              </div>
            </details>
          );
        })}

        {serverError !== null && (
          <div class="notice bad" role="alert" id="server-error" tabIndex={-1}>
            <p>
              <strong>{t("newRun.refused")}</strong> {serverError.message}
            </p>
          </div>
        )}

        <div class="actions sticky-actions">
          <button type="submit" class="button primary" disabled={busy || specs === null}>
            {busy ? t("newRun.starting") : t("newRun.start")}
          </button>
          <a class="button" href="#/engine">
            {t("newRun.cancel")}
          </a>
        </div>
      </form>
    </section>
  );
}

interface TextFieldProps {
  id: string;
  label: string;
  optional?: string;
  value: string;
  placeholder?: string;
  help?: string;
  error?: string;
  required?: boolean;
  onInput(value: string): void;
}

function TextField({ id, label, optional, value, placeholder, help, error, required, onInput }: TextFieldProps) {
  const describedBy = [help !== undefined ? `${id}-help` : "", error !== undefined ? `${id}-error` : ""].filter(Boolean).join(" ");
  return (
    <div class="field">
      <label class="field-label" for={id}>
        {label} {optional !== undefined && <span class={t("newRun.optional")}>{optional}</span>}
      </label>
      <input
        id={id}
        type="text"
        spellcheck={false}
        autocapitalize="off"
        autocomplete="off"
        value={value}
        placeholder={placeholder}
        required={required}
        aria-invalid={error !== undefined}
        aria-describedby={describedBy || undefined}
        onInput={(event) => onInput(event.currentTarget.value)}
      />
      {help !== undefined && (
        <span class="help" id={`${id}-help`}>
          {help}
        </span>
      )}
      {error !== undefined && (
        <span class="field-error" id={`${id}-error`}>
          {error}
        </span>
      )}
    </div>
  );
}

interface SettingFieldProps {
  spec: SettingSpec;
  value: string | boolean;
  error?: string;
  onChange(value: string | boolean): void;
}

function SettingField({ spec, value, error, onChange }: SettingFieldProps) {
  const id = `s-${spec.name}`;
  const changed = isChanged(spec, value);
  const fallback = defaultFormValue(spec);
  const describedBy = [`${id}-help`, error !== undefined ? `${id}-error` : ""].filter(Boolean).join(" ");
  const list = spec.kind === "integer_list" || spec.kind === "number_list";
  const numeric = spec.kind === "integer" || spec.kind === "number";
  return (
    <div class={`field setting${changed ? " changed" : ""}`}>
      {spec.kind === "boolean" ? (
        <label class="check" for={id}>
          <input
            id={id}
            type="checkbox"
            checked={value === true}
            aria-describedby={describedBy}
            aria-invalid={error !== undefined}
            onChange={(event) => onChange(event.currentTarget.checked)}
          />
          <span class="field-label">{humanise(spec.name)}</span>
        </label>
      ) : (
        <>
          <label class="field-label" for={id}>
            {humanise(spec.name)}
          </label>
          <input
            id={id}
            type="text"
            inputMode={spec.kind === "integer" ? "numeric" : numeric ? "decimal" : "text"}
            spellcheck={false}
            autocomplete="off"
            value={String(value)}
            aria-describedby={describedBy}
            aria-invalid={error !== undefined}
            onInput={(event) => onChange(event.currentTarget.value)}
          />
        </>
      )}
      <span class="help" id={`${id}-help`}>
        {engineText(spec.meaning)}
        {spec.meaning !== "" && !/[.!?]$/.test(spec.meaning) && "."}
        {list && t("newRun.commas")}
        {changed && (
          <>
            {" "}
            <button type="button" class="link" onClick={() => onChange(fallback)}>
              {t("newRun.defaultValue", { value: typeof fallback === "boolean" ? (fallback ? t("newRun.on") : t("newRun.off")) : fallback })}
            </button>
          </>
        )}
      </span>
      {error !== undefined && (
        <span class="field-error" id={`${id}-error`}>
          {error}
        </span>
      )}
    </div>
  );
}
