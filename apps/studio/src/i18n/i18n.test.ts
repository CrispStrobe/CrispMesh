import { afterEach, describe, expect, it } from "vitest";
import { format, formatParts, placeholders, type Message } from "./core";
import { AREAS, de, en, engineText, language, resolveLanguage, setLanguage, systemLanguage, t } from "./index";
import { formatBytes, formatDuration, formatNumber, formatPercent } from "../core/format";

const forms = (message: Message): string[] => (typeof message === "string" ? [message] : [message.one, message.other]);

describe("catalogues", () => {
  it("German has exactly the English keys", () => {
    expect(Object.keys(de).sort()).toEqual(Object.keys(en).sort());
  });

  it("every key carries its area's prefix, so merging the areas loses nothing", () => {
    let total = 0;
    for (const [area, catalogue] of Object.entries(AREAS)) {
      for (const key of Object.keys(catalogue.en)) expect(key.startsWith(`${area}.`), key).toBe(true);
      expect(Object.keys(catalogue.de).sort()).toEqual(Object.keys(catalogue.en).sort());
      total += Object.keys(catalogue.en).length;
    }
    expect(Object.keys(en)).toHaveLength(total);
  });

  it("both languages use the same placeholders and the same message shapes", () => {
    for (const key of Object.keys(en) as (keyof typeof en)[]) {
      const english: Message = en[key];
      const german: Message = de[key];
      expect(typeof german, key).toBe(typeof english);
      const wanted = new Set(forms(english).flatMap(placeholders));
      for (const text of forms(german)) {
        expect(new Set(placeholders(text)), key).toEqual(wanted);
        expect(text.trim(), key).not.toBe("");
      }
    }
  });
});

describe("messages", () => {
  afterEach(() => setLanguage("en"));

  it("is English until a language is chosen", () => {
    expect(language()).toBe("en");
    expect(t("app.nav.runs")).toBe("Runs");
  });

  it("switches to German and back", () => {
    setLanguage("de");
    expect(t("app.nav.runs")).toBe("Rekonstruktionen");
    setLanguage("en");
    expect(t("app.nav.runs")).toBe("Runs");
  });

  it("fills placeholders, numbers with the language's decimal sign", () => {
    expect(format("{a} of {b}", { a: 1.5, b: "x" }, "en")).toBe("1.5 of x");
    expect(format("{a} of {b}", { a: 1.5, b: "x" }, "de")).toBe("1,5 of x");
    expect(format("port {p}", { p: 8765 }, "de")).toBe("port 8765");
    expect(format("{missing}", {}, "en")).toBe("{missing}");
  });

  it("chooses plural forms by count", () => {
    const photos = { one: "{count} photo", other: "{count} photos" };
    expect(format(photos, { count: 1 }, "en")).toBe("1 photo");
    expect(format(photos, { count: 0 }, "en")).toBe("0 photos");
    const fotos = { one: "{count} Foto", other: "{count} Fotos" };
    expect(format(fotos, { count: 1 }, "de")).toBe("1 Foto");
    expect(format(fotos, { count: 3 }, "de")).toBe("3 Fotos");
  });

  it("puts elements between text parts", () => {
    const element = { element: true };
    expect(formatParts("see {link} now", { link: element }, "en")).toEqual(["see ", element, " now"]);
    expect(formatParts("{link}", { link: element }, "en")).toEqual([element]);
  });
});

describe("language choice", () => {
  afterEach(() => setLanguage("en"));

  it("follows the system language: German for de-*, else English", () => {
    expect(systemLanguage(["de-DE", "en"])).toBe("de");
    expect(systemLanguage(["de-AT"])).toBe("de");
    expect(systemLanguage(["en-GB", "de"])).toBe("en");
    expect(systemLanguage(["fr-FR", "de-CH"])).toBe("de");
    expect(systemLanguage(["fr-FR"])).toBe("en");
    expect(systemLanguage([])).toBe("en");
  });

  it("an explicit choice wins over the system language", () => {
    expect(resolveLanguage("system", ["de-DE"])).toBe("de");
    expect(resolveLanguage("en", ["de-DE"])).toBe("en");
    expect(resolveLanguage("de", ["en-US"])).toBe("de");
    expect(resolveLanguage("fr", ["en-US"])).toBe("en");
  });

  it("formats numbers for the active language", () => {
    expect(formatBytes(1690084)).toBe("1.7 MB");
    expect(formatDuration(2.02)).toBe("2.0 s");
    expect(formatPercent(0.123)).toBe("12.3%");
    setLanguage("de");
    expect(formatBytes(1690084)).toBe("1,7 MB");
    expect(formatDuration(2.02)).toBe("2,0 s");
    expect(formatNumber(0.5)).toBe("0,5000");
    expect(formatPercent(0.123)).toBe("12,3 %");
  });
});

it("translates known engine captions at render time without changing extension text", () => {
  setLanguage("de");
  expect(engineText("Turntable photos")).toBe("Drehteller-Fotos");
  expect(engineText("Cameras")).toBe("Kameras");
  expect(engineText("custom engine description")).toBe("custom engine description");
  setLanguage("en");
  expect(engineText("Cameras")).toBe("Cameras");
});


it("translates dense settings without changing their engine identifiers", async () => {
  const { default: schema } = await import("../../../../crates/dense/settings-schema.json");
  setLanguage("de");
  for (const spec of schema.settings) {
    expect(engineText(spec.name), spec.name).not.toBe(spec.name);
    expect(engineText(spec.meaning), spec.name).not.toBe(spec.meaning);
    expect(engineText(spec.group), spec.group).toBe(de[`settings.group.${spec.group}` as keyof typeof de]);
  }
  setLanguage("en");
  for (const spec of schema.settings) {
    expect(engineText(spec.name)).toBe(spec.name);
    expect(engineText(spec.meaning)).toBe(spec.meaning);
  }
});
