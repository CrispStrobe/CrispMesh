/**
 * Translations of the texts Studio composes itself (English and German).
 *
 * Catalogues live in `messages/<area>.ts`, one per part of the app; each exports `en` and a
 * `de: Catalogue<typeof en>`, so a missing or extra German key is a type error. Keys start with
 * the area's prefix (`app.`, `connection.`, ...). Placeholders are `{name}`; a message with
 * `{ one, other }` forms is chosen by the `count` parameter. i18n.test.ts checks that both
 * languages have the same keys and the same placeholders.
 *
 * Texts that come from the engine (its error messages, the settings schema's descriptions,
 * event messages) are shown as the engine sends them, in English.
 *
 * German glossary (keep to it):
 *   mesh -> Mesh (das Mesh, die Meshes)        photo(s) -> Foto(s)
 *   take photos / capture -> Fotos aufnehmen / Aufnahme
 *   turntable -> Drehteller                     run (a reconstruction job) -> Rekonstruktion
 *   engine -> Engine (die Engine)               neural surface -> neuronale Oberfläche
 *   2D splats -> 2D-Splats                      lens -> Objektiv; lens calibration -> Objektivkalibrierung
 *   field of view -> Bildwinkel                 mask -> Maske; depth (map) -> Tiefe(nkarte)
 *   camera(s) -> Kamera(s)                      inputs folder -> Eingabeordner
 *   settings -> Einstellungen                   recording / replay bundle -> Aufzeichnung
 *   texture -> Textur                           surface -> Oberfläche
 *   stage (of the pipeline) -> Schritt          level (of the image pyramid) -> Ebene
 *   example objects -> Beispielobjekte          access token -> Zugriffstoken
 *   download -> herunterladen / Download        scan (reference scan) -> Scan
 * Tone: buttons and labels as infinitives or nouns ("Verbinden", "Fotos aufnehmen"); sentences
 * that address the user use "Sie" consistently.
 */

import type { ComponentChild } from "preact";
import { format, formatParts, language, type Catalogue, type Params } from "./core";
import * as app from "./messages/app";
import * as connection from "./messages/connection";
import * as engine from "./messages/engine";
import * as inputs from "./messages/inputs";
import * as run from "./messages/run";

export const en = { ...app.en, ...connection.en, ...engine.en, ...inputs.en, ...run.en };
export const de: Catalogue<typeof en> = { ...app.de, ...connection.de, ...engine.de, ...inputs.de, ...run.de };

/** The areas, for the test that every key carries its area's prefix (so merging cannot overwrite). */
export const AREAS = { app, connection, engine, inputs, run } as const;

export type MessageKey = keyof typeof en;

const CATALOGUES = { en, de } as const;

/** The text of `key` in the active language, with `{name}` placeholders filled from `params`. */
export function t(key: MessageKey, params?: Params): string {
  return format(CATALOGUES[language()][key] ?? en[key], params);
}

/** Like `t`, but placeholders may be elements (a link, a code span); returns children for JSX. */
export function tParts(key: MessageKey, params: Record<string, string | number | ComponentChild>): ComponentChild[] {
  return formatParts<ComponentChild>(CATALOGUES[language()][key] ?? en[key], params);
}

export {
  formatLocaleDate,
  formatLocaleNumber,
  language,
  localDecimal,
  resolveLanguage,
  setLanguage,
  systemLanguage,
  type Language,
  type LanguageChoice,
} from "./core";
