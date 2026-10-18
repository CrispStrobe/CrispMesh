import { useEffect, useMemo, useState } from "preact/hooks";
import { groupSheets, type RunState, type Sheet } from "../core/reducer";
import type { RunSource } from "../sources/types";
import { Lightbox } from "./lightbox";
import { stageTitle } from "./stages";
import { t } from "../i18n";

interface Props {
  source: RunSource;
  run: RunState;
}

/** Resolves a run file to something an <img> can show (a plain URL, or an object URL when a token is needed). */
export function useImageUrl(source: RunSource, path: string): { url?: string; failed: boolean } {
  const [state, setState] = useState<{ path: string; url?: string; failed: boolean }>({ path, failed: false });
  useEffect(() => {
    let current = true;
    setState({ path, failed: false });
    source.imageUrl(path).then(
      (url) => current && setState({ path, url, failed: false }),
      () => current && setState({ path, failed: true }),
    );
    return () => {
      current = false;
    };
  }, [source, path]);
  return state.path === path ? state : { failed: false };
}

function SheetCard({ source, sheet, onOpen }: { source: RunSource; sheet: Sheet; onOpen(): void }) {
  const { url, failed } = useImageUrl(source, sheet.path);
  const [broken, setBroken] = useState(false);
  return (
    <li class="sheet">
      <button type="button" class="sheet-button" onClick={onOpen} aria-label={t("run.gallery.open", { label: sheet.label })}>
        <span class="sheet-frame">
          {url !== undefined && !broken && !failed ? (
            <img src={url} alt={sheet.label} loading="lazy" decoding="async" onError={() => setBroken(true)} />
          ) : (
            <span class="sheet-placeholder">{failed || broken ? t("run.image.unavailable") : t("run.image.loading")}</span>
          )}
        </span>
        <span class="sheet-label">{sheet.label}</span>
      </button>
    </li>
  );
}

export function Gallery({ source, run }: Props) {
  const groups = useMemo(() => groupSheets(run), [run.sheets, run.stages]);
  const ordered = useMemo(() => groups.flatMap((group) => group.sheets), [groups]);
  const [openPath, setOpenPath] = useState<string | null>(null);
  const openIndex = openPath === null ? -1 : ordered.findIndex((sheet) => sheet.path === openPath);

  return (
    <section class="panel gallery" aria-labelledby="gallery-heading">
      <div class="panel-head">
        <h2 id="gallery-heading">{t("run.gallery.heading")}</h2>
        <span class="sub" aria-live="polite">
          {ordered.length === 0 ? "" : t("run.gallery.sheets", { count: ordered.length })}
        </span>
      </div>
      {ordered.length === 0 && (
        <p class="sub">
          {run.status === "running" || run.status === "waiting"
            ? t("run.gallery.soon")
            : t("run.gallery.none")}
        </p>
      )}
      {groups.map((group) => (
        <div class="sheet-group" key={group.stage}>
          <h3>{stageTitle(group.stage)}</h3>
          <ul class="sheet-grid">
            {group.sheets.map((sheet) => (
              <SheetCard key={sheet.path} source={source} sheet={sheet} onOpen={() => setOpenPath(sheet.path)} />
            ))}
          </ul>
        </div>
      ))}
      {openIndex >= 0 && (
        <Lightbox
          source={source}
          sheets={ordered}
          index={openIndex}
          onIndex={(index) => setOpenPath(ordered[index]?.path ?? null)}
          onClose={() => setOpenPath(null)}
        />
      )}
    </section>
  );
}
