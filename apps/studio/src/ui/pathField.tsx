import type { ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { cleanPath, crumbs, joinPath, listable, parentPath, splitTyped } from "../core/paths";
import { describe, isAbort } from "../sources/transport";
import type { DataEntry, DataListing, Engine } from "../sources/types";
import { t } from "../i18n";

interface Props {
  id: string;
  label: string;
  optional?: string;
  value: string;
  placeholder?: string;
  help?: string;
  error?: string;
  required?: boolean;
  engine: Engine;
  /** `inputs`: pick a folder a run can start from. `file`: pick a file (or a folder). */
  want: "inputs" | "folder" | "file";
  onInput(value: string): void;
  /** Shown under the field's help, e.g. suggestions. */
  children?: ComponentChildren;
}

type Listing = { state: "loading" } | { state: "ready"; listing: DataListing } | { state: "failed"; message: string };

/** Fetches one folder of the engine's data directory; remembers what it has seen for this engine. */
function useListing(engine: Engine, path: string | null): Listing | null {
  const cache = useRef(new Map<string, DataListing>());
  const [result, setResult] = useState<{ path: string; listing: Listing } | null>(null);
  useEffect(() => {
    cache.current.clear();
  }, [engine]);
  useEffect(() => {
    if (path === null || engine.listData === undefined || !listable(path)) return;
    const known = cache.current.get(path);
    if (known !== undefined) {
      setResult({ path, listing: { state: "ready", listing: known } });
      return;
    }
    const abort = new AbortController();
    setResult({ path, listing: { state: "loading" } });
    engine.listData(path, abort.signal).then(
      (listing) => {
        cache.current.set(path, listing);
        if (!abort.signal.aborted) setResult({ path, listing: { state: "ready", listing } });
      },
      (problem) => {
        if (!isAbort(problem) && !abort.signal.aborted) {
          setResult({ path, listing: { state: "failed", message: describe(problem) } });
        }
      },
    );
    return () => abort.abort();
  }, [engine, path]);
  return result !== null && result.path === path ? result.listing : null;
}

/**
 * A path field with two helpers on top of plain typing: suggestions for the folder being
 * typed (a datalist, so the keyboard flow stays a text field), and a browser that walks
 * the engine's data directory and marks the folders a run can start from.
 */
export function PathField({ id, label, optional, value, placeholder, help, error, required, engine, want, onInput, children }: Props) {
  const canList = engine.listData !== undefined;
  const [open, setOpen] = useState(false);
  const [folder, setFolder] = useState("");
  const typed = splitTyped(/^(\/|[A-Za-z]:)/.test(value.trim()) ? "" : value);
  // Type-ahead: list the folder being typed in, a moment after the typing pauses.
  const [suggestIn, setSuggestIn] = useState<string | null>(null);
  useEffect(() => {
    if (!canList) return;
    const timer = setTimeout(() => setSuggestIn(typed.parent), 200);
    return () => clearTimeout(timer);
  }, [typed.parent, canList]);
  const suggestions = useListing(engine, suggestIn);
  const browsing = useListing(engine, open ? folder : null);

  const describedBy = [help !== undefined ? `${id}-help` : "", error !== undefined ? `${id}-error` : ""].filter(Boolean).join(" ");
  const options =
    suggestions?.state === "ready" && suggestIn === typed.parent
      ? suggestions.listing.entries.filter((entry) => entry.directory || want === "file")
      : [];

  const toggle = () => {
    if (!open) {
      // Start where the field points: the typed folder if it is one, else its parent.
      // An absolute path (chosen with the native picker) is not inside the data folder: start at its top.
      const absolute = /^(\/|[A-Za-z]:)/.test(value.trim());
      const clean = absolute ? "" : cleanPath(value);
      setFolder(listable(clean) ? (value.trim().endsWith("/") || clean === "" ? clean : parentPath(clean)) : "");
    }
    setOpen(!open);
  };

  const choose = (path: string) => {
    onInput(path);
    setOpen(false);
    document.getElementById(id)?.focus();
  };

  const visible = (entry: DataEntry) => entry.directory || want === "file";

  return (
    <div class="field">
      <label class="field-label" for={id}>
        {label} {optional !== undefined && <span class="optional">{optional}</span>}
      </label>
      <div class="path-row">
        <input
          id={id}
          type="text"
          spellcheck={false}
          autocapitalize="off"
          autocomplete="off"
          list={canList ? `${id}-list` : undefined}
          value={value}
          placeholder={placeholder}
          required={required}
          aria-invalid={error !== undefined}
          aria-describedby={describedBy || undefined}
          onInput={(event) => onInput(event.currentTarget.value)}
        />
        {canList && (
          <button type="button" class="button" aria-expanded={open} aria-controls={`${id}-browser`} onClick={toggle}>
            {t("inputs.path.browse")}
          </button>
        )}
        {engine.pickPath !== undefined && (
          <button
            type="button"
            class="button"
            onClick={() => {
              void engine.pickPath?.(want === "file" ? "file" : "folder", label).then(
                (chosen) => {
                  if (chosen !== null) onInput(chosen);
                },
                () => undefined,
              );
            }}
          >
            {t("inputs.path.choose")}<span class="visually-hidden">{t("inputs.path.chooseHidden", { label })}</span>
          </button>
        )}
      </div>
      {canList && (
        <datalist id={`${id}-list`}>
          {options.map((entry) => (
            <option key={entry.name} value={joinPath(typed.parent, entry.name) + (entry.directory && !entry.inputs ? "/" : "")}>
              {t(entry.inputs ? "inputs.path.kind.inputs" : entry.directory ? "inputs.path.kind.folder" : "inputs.path.kind.file")}
            </option>
          ))}
        </datalist>
      )}
      {help !== undefined && (
        <span class="help" id={`${id}-help`}>
          {help}
        </span>
      )}
      {children}
      {error !== undefined && (
        <span class="field-error" id={`${id}-error`}>
          {error}
        </span>
      )}
      {open && (
        <div class="browser" id={`${id}-browser`} role="group" aria-label={t("inputs.path.browserLabel", { label })}>
          <nav class="browser-crumbs" aria-label={t("inputs.path.crumbsLabel")}>
            <button type="button" class="link" onClick={() => setFolder("")} aria-current={folder === "" ? "location" : undefined}>
              {t("inputs.path.dataFolder")}
            </button>
            {crumbs(folder).map((crumb) => (
              <span key={crumb.path}>
                <span aria-hidden="true"> / </span>
                <button
                  type="button"
                  class="link"
                  onClick={() => setFolder(crumb.path)}
                  aria-current={crumb.path === folder ? "location" : undefined}
                >
                  {crumb.name}
                </button>
              </span>
            ))}
          </nav>
          {(browsing === null || browsing.state === "loading") && <p class="sub">{t("inputs.path.loading")}</p>}
          {browsing?.state === "failed" && (
            <p class="field-error" role="alert">
              {t("inputs.path.cannotList", { message: browsing.message })}
            </p>
          )}
          {browsing?.state === "ready" && (
            <ul class="browser-list">
              {browsing.listing.entries.filter(visible).length === 0 && (
                <li class="sub browser-empty">{want === "inputs" ? t("inputs.path.noFolders") : t("inputs.path.nothing")}</li>
              )}
              {browsing.listing.entries.filter(visible).map((entry) => {
                const path = joinPath(folder, entry.name);
                const pick = want === "inputs" ? entry.inputs : want === "folder" ? entry.directory : !entry.directory;
                return (
                  <li key={entry.name} class={pick ? "pickable" : undefined}>
                    {entry.directory ? (
                      <button type="button" class="browser-entry" onClick={() => setFolder(path)}>
                        <span class="entry-kind" aria-hidden="true">
                          {"▸"}
                        </span>
                        <span class="entry-name">{entry.name}</span>
                        {entry.inputs && <span class="badge status-complete">{t("inputs.path.badge.inputs")}</span>}
                        {entry.photos && <span class="badge status-running">{t("inputs.path.badge.photos")}</span>}
                        <span class="visually-hidden">{t("inputs.path.openFolder")}</span>
                      </button>
                    ) : (
                      <button type="button" class="browser-entry" onClick={() => choose(path)}>
                        <span class="entry-kind" aria-hidden="true">
                          {"·"}
                        </span>
                        <span class="entry-name">{entry.name}</span>
                        {entry.calibration && <span class="badge status-running">{t("inputs.path.badge.calibration")}</span>}
                        <span class="visually-hidden">{t("inputs.path.useFile")}</span>
                      </button>
                    )}
                    {entry.directory && pick && (
                      <button type="button" class="button primary small" onClick={() => choose(path)}>
                        {t("inputs.path.use")}<span class="visually-hidden"> {entry.name}</span>
                      </button>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
          <div class="browser-foot">
            <span class="sub">
              {t(want === "inputs" ? "inputs.path.foot.inputs" : want === "folder" ? "inputs.path.foot.folder" : "inputs.path.foot.file")}
            </span>
            <button type="button" class="button small" onClick={() => setOpen(false)}>
              {t("inputs.path.close")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
