/**
 * The language machinery: the active language, message lookup with `{name}` placeholders and
 * simple plurals, and number formatting in the active language. No dependency.
 *
 * Until `setLanguage` is called (main.tsx does it from the remembered choice) the language is
 * English, so tests and headless checks see English text.
 */

export type Language = "en" | "de";
export type LanguageChoice = "system" | Language;

/** A message with a singular and a plural form, chosen by the `count` parameter (Intl plural rules). */
export interface Plural {
  readonly one: string;
  readonly other: string;
}
export type Message = string | Plural;

/** The German (or any other) catalogue for an English one: the same keys, the same shapes. */
export type Catalogue<E> = { [K in keyof E]: E[K] extends string ? string : Plural };

export const LANGUAGES: readonly Language[] = ["en", "de"];

let active: Language = "en";

export function language(): Language {
  return active;
}

/** Makes `next` the active language and tells the document (for screen readers and hyphenation). */
export function setLanguage(next: Language): void {
  active = next;
  if (typeof document !== "undefined") document.documentElement.lang = next;
}

/** German for the first supported language the system or browser prefers (`de`, `de-AT`, ...), else English. */
export function systemLanguage(preferred?: readonly string[]): Language {
  const list =
    preferred ?? (typeof navigator !== "undefined" ? (navigator.languages?.length ? navigator.languages : [navigator.language ?? ""]) : []);
  for (const tag of list) {
    const primary = tag.toLowerCase().split(/[-_]/)[0];
    if (primary === "de") return "de";
    if (primary === "en") return "en";
  }
  return "en";
}

/** The language a choice stands for. Unknown stored values count as "system". */
export function resolveLanguage(choice: string, preferred?: readonly string[]): Language {
  return choice === "en" || choice === "de" ? choice : systemLanguage(preferred);
}

export type Params = Record<string, string | number>;

const pluralRules = new Map<Language, Intl.PluralRules>();
function pluralCategory(count: number, lang: Language): "one" | "other" {
  let rules = pluralRules.get(lang);
  if (rules === undefined) {
    rules = new Intl.PluralRules(lang);
    pluralRules.set(lang, rules);
  }
  return rules.select(count) === "one" ? "one" : "other";
}

/** A number as a message parameter: the active language's decimal sign, no grouping (ports, years, ids stay intact). */
function paramText(value: string | number, lang: Language): string {
  if (typeof value === "string") return value;
  if (!Number.isFinite(value)) return String(value);
  return new Intl.NumberFormat(lang, { useGrouping: false, maximumFractionDigits: 20 }).format(value);
}

/** The placeholders `{name}` of a message text, in order (used by the catalogue test). */
export function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]!);
}

/** Picks the form of `message` and fills its placeholders. A missing parameter leaves `{name}` in place. */
export function format(message: Message, params: Params | undefined, lang: Language = active): string {
  let text: string;
  if (typeof message === "string") text = message;
  else {
    const count = params?.count;
    text = typeof count === "number" ? message[pluralCategory(count, lang)] : message.other;
  }
  if (params === undefined) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => (name in params ? paramText(params[name]!, lang) : whole));
}

/** Splits a filled message around placeholders whose values are not text (elements), keeping their order. */
export function formatParts<T>(message: Message, params: Record<string, string | number | T>, lang: Language = active): (string | T)[] {
  const plain: Params = {};
  for (const [name, value] of Object.entries(params)) if (typeof value === "string" || typeof value === "number") plain[name] = value;
  const text = format(message, plain, lang);
  const parts: (string | T)[] = [];
  let last = 0;
  for (const match of text.matchAll(/\{(\w+)\}/g)) {
    const name = match[1]!;
    if (!(name in params)) continue;
    if (match.index! > last) parts.push(text.slice(last, match.index));
    parts.push(params[name] as T);
    last = match.index! + match[0].length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

/** A number in the active language (Intl). */
export function formatLocaleNumber(value: number, options?: Intl.NumberFormatOptions): string {
  return new Intl.NumberFormat(active, options).format(value);
}

/** A date (and by default time) in the active language (Intl). */
export function formatLocaleDate(date: Date | number, options: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" }): string {
  return new Intl.DateTimeFormat(active, options).format(date);
}

/** A number already written with "." as decimal sign, in the active language ("1.5" -> "1,5" in German). */
export function localDecimal(text: string): string {
  return active === "de" ? text.replace(/(\d)\.(\d)/g, "$1,$2") : text;
}
