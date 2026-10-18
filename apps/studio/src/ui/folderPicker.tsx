import { useRef, useState } from "preact/hooks";
import { describeInputs, megapixelText, previewAdvice, relativeFiles, type BrowserEngine, type PickedInputs } from "../sources/browserEngine";
import { describe } from "../sources/transport";
import { t } from "../i18n";

interface Props {
  engine: BrowserEngine;
  error?: string;
  /** Called with the folder's name once it is accepted, or "" when it is not usable. */
  onPicked(name: string): void;
}

interface DirectoryHandle {
  name: string;
  values(): AsyncIterable<{ kind: "file" | "directory"; name: string; getFile?(): Promise<File> } & Partial<DirectoryHandle>>;
}

async function walk(handle: DirectoryHandle, prefix: string, out: { path: string; file: File }[]): Promise<void> {
  for await (const entry of handle.values()) {
    if (entry.kind === "file" && entry.getFile !== undefined) out.push({ path: `${prefix}/${entry.name}`, file: await entry.getFile() });
    else if (entry.kind === "directory" && entry.values !== undefined) await walk(entry as DirectoryHandle, `${prefix}/${entry.name}`, out);
  }
}

/**
 * Picks an inputs folder on this device for the engine in the browser. Uses the File
 * System Access API where the browser has it, and a folder input everywhere else. The
 * files are read here when the engine asks for them; nothing leaves the device.
 */
export function FolderPicker({ engine, error, onPicked }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [picked, setPicked] = useState<PickedInputs | null>(() => engine.inputs());
  const [previews, setPreviews] = useState<boolean | null>(null);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const picker = (window as unknown as { showDirectoryPicker?: () => Promise<DirectoryHandle> }).showDirectoryPicker;

  const accept = async (name: string, entries: { path: string; file: File }[]) => {
    setProblem("");
    try {
      const files = relativeFiles(entries);
      const cameras = files.find(([path]) => path === "cameras.json");
      if (cameras === undefined) throw new Error(t("inputs.folder.noCameras", { name }));
      const described = describeInputs(name, files, await cameras[1].text());
      engine.setInputs(described, null);
      setPicked(described);
      setPreviews(null);
      onPicked(name);
    } catch (reason) {
      engine.setInputs(null);
      setPicked(null);
      setProblem(describe(reason));
      onPicked("");
    }
  };

  const choose = async () => {
    if (picker === undefined) {
      input.current?.click();
      return;
    }
    setBusy(true);
    try {
      const handle = await picker.call(window);
      const entries: { path: string; file: File }[] = [];
      await walk(handle, handle.name, entries);
      await accept(handle.name, entries);
    } catch (reason) {
      // Closing the dialog is not an error.
      if (!(reason instanceof DOMException && reason.name === "AbortError")) setProblem(describe(reason));
    } finally {
      setBusy(false);
    }
  };

  const advice = picked === null ? null : previewAdvice(picked.views, picked.megapixels);
  const shown = previews ?? advice?.on ?? false;

  return (
    <div class="field">
      <span class="field-label" id="f-inputs-label">
        {t("inputs.folder.label")}
      </span>
      <div class="path-row">
        <button type="button" class="button" onClick={() => void choose()} disabled={busy} aria-describedby="f-inputs-help">
          {busy ? t("inputs.folder.reading") : picked === null ? t("inputs.folder.choose") : t("inputs.folder.chooseAnother")}
        </button>
        {/* Always present: the fallback where there is no folder dialog API, and what automated tests use. */}
        <input
          ref={input}
          id="f-inputs-files"
          class="visually-hidden"
          type="file"
          multiple
          aria-labelledby="f-inputs-label"
          tabIndex={-1}
          {...{ webkitdirectory: true }}
          onChange={(event) => {
            const list = [...(event.currentTarget.files ?? [])];
            const entries = list.map((file) => ({ path: (file as File & { webkitRelativePath?: string }).webkitRelativePath || file.name, file }));
            const name = entries[0]?.path.split("/")[0] ?? "";
            if (entries.length > 0) void accept(name, entries);
          }}
        />
      </div>
      <span class="help" id="f-inputs-help">
        {t("inputs.folder.help")}
      </span>
      {picked !== null && advice !== null && (
        <>
          <span class="picked" role="status">
            <strong>{picked.name}</strong>: {t("inputs.folder.picked", { files: picked.files.length, photos: picked.views, megapixels: megapixelText(picked.megapixels) })}
          </span>
          <label class="check" for="f-previews">
            <input
              id="f-previews"
              type="checkbox"
              checked={shown}
              aria-describedby="f-previews-help"
              onChange={(event) => {
                setPreviews(event.currentTarget.checked);
                engine.setInputs(picked, event.currentTarget.checked);
              }}
            />
            <span>{t("inputs.folder.previews")}</span>
          </label>
          <span class="help" id="f-previews-help">
            {advice.reason}
            {previews !== null && previews !== advice.on && (previews ? t("inputs.folder.previewsOn") : t("inputs.folder.previewsOff"))}
          </span>
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
