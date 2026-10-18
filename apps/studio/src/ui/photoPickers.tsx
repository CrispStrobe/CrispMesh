import { useEffect, useRef, useState } from "preact/hooks";
import { AUTOMATIC_LENS, checkCalibration, photoFiles, type BrowserEngine, type PickedPhotos } from "../sources/browserEngine";
import { describe } from "../sources/transport";
import { CaptureScreen } from "./capture";
import { importFolderName } from "./deviceImport";
import { t, tParts } from "../i18n";

interface Props {
  engine: BrowserEngine;
  error?: string;
  /** Called with a name once something usable is chosen, or "" when it is not. */
  onPicked(name: string): void;
  /** Photos taken with the camera come without a lens file: called when the automatic lens was chosen for them. */
  onAutomaticLens?(label: string): void;
  /** Called with the number of photos chosen, or null when none are. */
  onCount?(count: number | null): void;
  onTurns?(sizes: number[]): void;
  /** The calibration field's value, so a choice made elsewhere shows here. */
  value?: string;
}

/** The photos for a run in this browser: several image files, or a folder of them. */
export function PhotosPicker({ engine, error, onPicked, onAutomaticLens, onCount, onTurns }: Props) {
  const files = useRef<HTMLInputElement>(null);
  const folder = useRef<HTMLInputElement>(null);
  const [capturing, setCapturing] = useState(false);
  const [picked, setPicked] = useState<PickedPhotos | null>(() => engine.pickedPhotos());
  const [previews, setPreviews] = useState(false);
  const [problem, setProblem] = useState("");

  const accept = (list: File[], name: string) => {
    onTurns?.([]);
    const photos = photoFiles(list.map((file) => ({ name: file.name, file })));
    if (photos.length < 3) {
      setProblem(photos.length === 0 ? t("inputs.photos.noPhotos") : t("inputs.photos.tooFew"));
      engine.setPhotos(null);
      setPicked(null);
      onPicked("");
      onCount?.(null);
      return;
    }
    const chosen = { name, files: photos };
    setProblem("");
    engine.setPhotos(chosen, previews);
    setPicked(chosen);
    onPicked(name);
    onCount?.(photos.length);
  };

  const megabytes = picked === null ? 0 : picked.files.reduce((sum, [, file]) => sum + file.size, 0) / 1e6;
  return (
    <div class="field">
      <span class="field-label" id="f-photos-label">
        {t("inputs.photos.label")}
      </span>
      <div class="path-row">
        <button type="button" class="button" onClick={() => files.current?.click()}>
          {t("inputs.photos.choose")}
        </button>
        <button type="button" class="button" onClick={() => folder.current?.click()}>
          {t("inputs.photos.chooseFolder")}
        </button>
        <button type="button" class="button" onClick={() => setCapturing(true)}>
          {t("inputs.photos.take")}
        </button>
        <input
          ref={files}
          id="f-photos-files"
          class="visually-hidden"
          type="file"
          multiple
          accept="image/png,image/jpeg"
          tabIndex={-1}
          aria-labelledby="f-photos-label"
          onChange={(event) => {
            const list = [...(event.currentTarget.files ?? [])];
            event.currentTarget.value = "";
            if (list.length > 0) accept(list, "photos");
          }}
        />
        <input
          ref={folder}
          id="f-photos-folder"
          class="visually-hidden"
          type="file"
          multiple
          tabIndex={-1}
          aria-labelledby="f-photos-label"
          {...{ webkitdirectory: true }}
          onChange={(event) => {
            const list = [...(event.currentTarget.files ?? [])];
            event.currentTarget.value = "";
            const first = list[0] as (File & { webkitRelativePath?: string }) | undefined;
            if (list.length > 0) accept(list, first?.webkitRelativePath?.split("/")[0] || "photos");
          }}
        />
      </div>
      <span class="help">
        {t("inputs.photos.help")}
      </span>
      {capturing && (
        <CaptureScreen
          onCancel={() => setCapturing(false)}
          onDone={(taken, _frame, sizes) => {
            setCapturing(false);
            accept(taken, importFolderName(new Date()).replace("photos-", "capture-"));
            onTurns?.(sizes);
            // A camera of this device has no lens file here: the focal length is found from the turn.
            if (engine.pickedCalibration() === null) {
              engine.useAutomaticLens();
              onAutomaticLens?.(AUTOMATIC_LENS);
            }
          }}
        />
      )}
      {picked !== null && (
        <>
          <span class="picked" role="status">
            <strong>{picked.name}</strong>:{" "}
            {t("inputs.photos.picked", {
              count: picked.files.length,
              megabytes: Math.round(megabytes),
              first: picked.files[0]![0],
              last: picked.files[picked.files.length - 1]![0],
            })}
          </span>
          <label class="check" for="f-photo-previews">
            <input
              id="f-photo-previews"
              type="checkbox"
              checked={previews}
              onChange={(event) => {
                setPreviews(event.currentTarget.checked);
                engine.setPhotos(picked, event.currentTarget.checked);
              }}
            />
            <span>{t("inputs.photos.previews")}</span>
          </label>
          <span class="help">{t("inputs.photos.previewsHelp")}</span>
        </>
      )}
      {(problem !== "" || error !== undefined) && (
        <span class="field-error" role="alert">
          {problem !== "" ? problem : error}
        </span>
      )}
    </div>
  );
}

/** The lens calibration for a run in this browser: one that comes with the app, or a file from this device. */
export function CalibrationPicker({ engine, error, onPicked, value }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [shipped, setShipped] = useState<{ label: string; path: string }[]>([]);
  const [current, setCurrent] = useState(() => engine.pickedCalibration()?.label ?? "");
  useEffect(() => {
    if (value !== undefined && value !== "" && value !== current) setCurrent(value);
  }, [value]);
  const [problem, setProblem] = useState("");
  useEffect(() => {
    let alive = true;
    void engine.calibrations().then((list) => alive && setShipped(list));
    return () => {
      alive = false;
    };
  }, [engine]);
  const done = (label: string) => {
    setProblem("");
    setCurrent(label);
    onPicked(label);
  };
  const fail = (reason: unknown) => {
    engine.setCalibration(null);
    setCurrent("");
    setProblem(describe(reason));
    onPicked("");
  };
  return (
    <div class="field">
      <span class="field-label" id="f-calibration-label">
        {t("inputs.lens.label")}
      </span>
      <div class="path-row">
        <button type="button" class="button" onClick={() => input.current?.click()}>
          {t("inputs.lens.chooseFile")}
        </button>
        <input
          ref={input}
          id="f-calibration-file"
          class="visually-hidden"
          type="file"
          accept=".json,application/json"
          tabIndex={-1}
          aria-labelledby="f-calibration-label"
          onChange={(event) => {
            const file = event.currentTarget.files?.[0];
            event.currentTarget.value = "";
            if (file === undefined) return;
            file
              .text()
              .then((text) => {
                engine.setCalibration({ label: file.name, text: checkCalibration(text) });
                done(file.name);
              })
              .catch(fail);
          }}
        />
      </div>
      {shipped.length > 0 && (
        <div class="calibrations" role="group" aria-label={t("inputs.lens.includedLabel")}>
          <span class="help">{t("inputs.lens.included")}</span>
          {shipped.map((calibration) => {
            const name = calibration.path.split("/").pop() ?? calibration.path;
            return (
              <button
                key={calibration.path}
                type="button"
                class="button small"
                aria-pressed={current === name}
                onClick={() => void engine.useShippedCalibration(calibration.path).then(() => done(name), fail)}
              >
                {calibration.label}
              </button>
            );
          })}
        </div>
      )}
      <div class="calibrations" role="group" aria-label={t("inputs.lens.withoutLabel")}>
        <span class="help">{t("inputs.lens.noFile")}</span>
        <button
          type="button"
          class="button small"
          aria-pressed={current === AUTOMATIC_LENS}
          onClick={() => {
            engine.useAutomaticLens();
            done(AUTOMATIC_LENS);
          }}
        >
          {t("inputs.lens.automatic")}
        </button>
      </div>
      <span class="help">
        {t("inputs.lens.help")}
      </span>
      {current !== "" && (
        <span class="picked" role="status">
          {tParts("inputs.lens.picked", { name: <strong>{current === AUTOMATIC_LENS ? t("inputs.lens.automaticLabel") : current}</strong> })}
        </span>
      )}
      {(problem !== "" || error !== undefined) && (
        <span class="field-error" role="alert">
          {problem !== "" ? problem : error}
        </span>
      )}
    </div>
  );
}
