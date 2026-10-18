import { engineText } from "../i18n";
import { useEffect, useState } from "preact/hooks";
import { isChanged, optionError, type OptionSpec, type OptionValues } from "../core/providerOptions";
import { chosenProviders, type StartPoint } from "../core/startPoints";
import type { Engine } from "../sources/types";
import { PathField } from "./pathField";
import { language, t } from "../i18n";

interface Props {
  engine: Engine;
  point: StartPoint;
  /** Chosen provider by module. */
  providers: Record<string, string>;
  onProvider(module: string, id: string): void;
  options: OptionValues;
  onOption(name: string, value: string | boolean): void;
  values: Record<string, string>;
  onValue(key: string, value: string): void;
  /** Error for a field key (paths) or an option name. */
  error(key: string): string | undefined;
  /** Where the tools of unavailable providers are set up, if this app has such a screen. */
  toolsHref?: string;
}

function OptionField({ spec, value, error, onChange }: { spec: OptionSpec; value: string | boolean | undefined; error?: string; onChange(value: string | boolean): void }) {
  const id = `o-${spec.name}`;
  const describedBy = [`${id}-help`, error !== undefined ? `${id}-error` : ""].filter(Boolean).join(" ");
  const text = value === undefined ? spec.default : String(value);
  const changed = isChanged(spec, value);
  return (
    <div class={`field setting${changed ? " changed" : ""}`}>
      {spec.kind === "switch" ? (
        <label class="check" for={id}>
          <input
            id={id}
            type="checkbox"
            checked={value === undefined ? spec.default === "on" : value === true}
            aria-describedby={describedBy}
            onChange={(event) => onChange(event.currentTarget.checked)}
          />
          <span class="field-label">{engineText(spec.label)}</span>
        </label>
      ) : (
        <>
          <label class="field-label" for={id}>
            {engineText(spec.label)}
          </label>
          {spec.kind === "choice" ? (
            <select id={id} value={text} aria-describedby={describedBy} onChange={(event) => onChange(event.currentTarget.value)}>
              {spec.choices.map((choice) => (
                <option key={choice} value={choice}>
                  {choice}
                </option>
              ))}
            </select>
          ) : (
            <input
              id={id}
              type="text"
              inputMode={spec.kind === "integer" ? "numeric" : spec.kind === "number" ? "decimal" : "text"}
              spellcheck={false}
              autocomplete="off"
              value={text}
              aria-invalid={error !== undefined}
              aria-describedby={describedBy}
              onInput={(event) => onChange(event.currentTarget.value)}
            />
          )}
        </>
      )}
      <span class="help" id={`${id}-help`}>
        <span class="mono">--{spec.name}</span>
        {changed && spec.default !== "" && t("inputs.provider.default", { value: spec.default })}
      </span>
      {error !== undefined && (
        <span class="field-error" id={`${id}-error`}>
          {error}
        </span>
      )}
    </div>
  );
}

function OptionGrid({ specs, options, onOption, error }: { specs: OptionSpec[]; options: OptionValues; onOption(name: string, value: string | boolean): void; error(key: string): string | undefined }) {
  return (
    <div class="settings-grid">
      {specs.map((spec) => (
        <OptionField key={spec.name} spec={spec} value={options[spec.name]} error={error(spec.name) ?? optionError(spec, options[spec.name])} onChange={(value) => onOption(spec.name, value)} />
      ))}
    </div>
  );
}

function More({ title, specs, options, onOption, error }: { title: string; specs: OptionSpec[]; options: OptionValues; onOption(name: string, value: string | boolean): void; error(key: string): string | undefined }) {
  if (specs.length === 0) return null;
  const changed = specs.filter((spec) => isChanged(spec, options[spec.name])).length;
  return (
    <details class="raw provider-more">
      <summary>
        {title}
        {changed > 0 ? t("inputs.provider.changed", { count: changed }) : ""}
      </summary>
      <OptionGrid specs={specs} options={options} onOption={onOption} error={error} />
    </details>
  );
}

/**
 * For a start point with interchangeable modules (masks, cameras): which provider does each,
 * with the ones that cannot run shown and explained, the chosen provider's paths and
 * options, and the options that belong to no provider. All of it from the engine's description.
 */
export function ProviderChoices({ engine, point, providers, onProvider, options, onOption, values, onValue, error, toolsHref }: Props) {
  const chosen = chosenProviders(point, providers);
  return (
    <>
      {point.providers.map((choice) => {
        const current = choice.options.find((option) => option.id === chosen[choice.module]) ?? choice.options[0]!;
        return (
          <fieldset class="provider" key={`${point.id}-${choice.module}`}>
            <legend>{engineText(choice.label)}</legend>
            <div class="provider-list" role="radiogroup" aria-label={engineText(choice.label)}>
              {choice.options.map((option) => (
                <label key={option.id} class={`provider-option${option.id === current.id ? " chosen" : ""}${option.available ? "" : " unavailable"}`}>
                  <input
                    type="radio"
                    name={`provider-${choice.module}`}
                    value={option.id}
                    checked={option.id === current.id}
                    disabled={!option.available}
                    onChange={() => onProvider(choice.module, option.id)}
                  />
                  <span class="provider-text">
                    <span class="provider-name">
                      {engineText(option.label)}
                      {!option.available && <span class="badge status-cancelled">{t("inputs.provider.notAvailable")}</span>}
                    </span>
                    {option.meaning !== undefined && <span class="help">{engineText(option.meaning)}</span>}
                    {option.external.length > 0 && (
                      <span class="help">
                        {t("inputs.provider.needs", { list: option.external.join(", ") })}
                        {option.available && option.version !== undefined && t("inputs.provider.found", { version: option.version })}
                        {option.license !== undefined && t("inputs.provider.license", { license: option.license })}
                      </span>
                    )}
                    {!option.available && (
                      <span class="field-error">
                        {engineText(option.reason) ?? t("inputs.provider.cannotUse")}{" "}
                        {toolsHref !== undefined && option.external.length > 0 && <a href={toolsHref}>{t("inputs.provider.tools")}</a>}
                      </span>
                    )}
                  </span>
                </label>
              ))}
            </div>
            {current.inputs.map((input) => (
              <PathField
                key={input.key}
                engine={engine}
                want={input.kind}
                id={`f-${input.key}`}
                label={engineText(input.label)}
                value={values[input.key] ?? ""}
                onInput={(value) => onValue(input.key, value)}
                help={engineText(input.help)}
                error={error(input.key)}
                required
              />
            ))}
            {current.settings.length > 0 && <OptionGrid specs={current.settings} options={options} onOption={onOption} error={error} />}
            <More title={t("inputs.provider.more", { module: language() === "en" ? choice.label.toLowerCase() : choice.label })} specs={choice.settings.filter((spec) => !current.settings.some((own) => own.name === spec.name))} options={options} onOption={onOption} error={error} />
          </fieldset>
        );
      })}
      {point.optionGroups.map((group) => (
        <More key={group.id} title={engineText(group.label)} specs={group.settings} options={options} onOption={onOption} error={error} />
      ))}
    </>
  );
}

/** Calibration files the engine knows about, offered under the calibration field. */
export function CalibrationList({ engine, current, onPick }: { engine: Engine; current: string; onPick(path: string): void }) {
  const [found, setFound] = useState<{ label: string; path: string }[]>([]);
  useEffect(() => {
    let alive = true;
    engine.calibrations?.().then(
      (list) => alive && setFound(list),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [engine]);
  if (found.length === 0) return null;
  return (
    <div class="calibrations" role="group" aria-label={t("inputs.lens.foundLabel")}>
      <span class="help">{t("inputs.lens.found")}</span>
      {found.map((calibration) => (
        <button key={calibration.path} type="button" class="button small" aria-pressed={calibration.path === current} title={calibration.path} onClick={() => onPick(calibration.path)}>
          {calibration.label}
        </button>
      ))}
    </div>
  );
}
