import { useRef, useState } from "preact/hooks";
import type { Engine } from "../sources/types";
import { describe } from "../sources/transport";
import { t } from "../i18n";

interface Props {
  engine: Engine;
  /** `photos`: several images, into a new folder. `calibration`: one lens file. */
  what: "photos" | "calibration";
  /** Called with the imported folder or file, relative to the data folder, and the number of files copied. */
  onImported(path: string, count: number): void;
}

/** `photos-20261006-1530`: a new folder per import, so two imports never mix. */
export function importFolderName(now: Date): string {
  const two = (n: number) => String(n).padStart(2, "0");
  return `photos-${now.getFullYear()}${two(now.getMonth() + 1)}${two(now.getDate())}-${two(now.getHours())}${two(now.getMinutes())}${two(now.getSeconds())}`;
}

/**
 * On a phone, where the app has no folder dialog of its own: photos and a lens
 * calibration come in through the system's picker (Photos or Files) and are copied into
 * the app's data folder, which the Files app also shows.
 */
export function DeviceImport({ engine, what, onImported }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [state, setState] = useState<{ busy: boolean; message: string; bad: boolean }>({ busy: false, message: "", bad: false });
  if (engine.importFiles === undefined) return null;
  const importFiles = engine.importFiles.bind(engine);

  const take = async (files: File[]) => {
    if (files.length === 0) return;
    if (what === "photos" && files.length < 3) {
      setState({ busy: false, message: t("connection.import.atLeastThree"), bad: true });
      return;
    }
    setState({ busy: true, message: t("connection.import.copying", { count: files.length }), bad: false });
    try {
      const folder = what === "photos" ? importFolderName(new Date()) : "calibrations";
      const last = await importFiles(folder, files, (done, total) => setState({ busy: true, message: t("connection.import.progress", { done, total }), bad: false }));
      onImported(what === "photos" ? folder : last, files.length);
      setState({ busy: false, message: what === "photos" ? t("connection.import.photosDone", { count: files.length, folder }) : t("connection.import.fileDone", { name: files[0]!.name }), bad: false });
    } catch (problem) {
      setState({ busy: false, message: describe(problem), bad: true });
    }
  };

  return (
    <div class="device-import">
      <button type="button" class="button" disabled={state.busy} onClick={() => input.current?.click()}>
        {what === "photos" ? t("connection.import.choosePhotos") : t("connection.import.chooseCalibration")}
      </button>
      <input
        ref={input}
        class="visually-hidden"
        type="file"
        tabIndex={-1}
        aria-hidden="true"
        multiple={what === "photos"}
        accept={what === "photos" ? "image/*" : ".json,application/json"}
        onChange={(event) => {
          const files = [...(event.currentTarget.files ?? [])];
          event.currentTarget.value = "";
          void take(files);
        }}
      />
      {state.message !== "" && (
        <span class={state.bad ? "field-error" : "help"} role="status">
          {state.message}
        </span>
      )}
    </div>
  );
}
