import { useEffect, useState } from "preact/hooks";
import type { Inspection } from "../core/reducer";
import type { RunSource } from "../sources/types";
import { useImageUrl } from "./gallery";
import { engineText, t } from "../i18n";

interface Props {
  source: RunSource;
  steps: Inspection[];
}

/**
 * The inspection sheets, one per step, as a flip-through: the same views and the same detail
 * crops after every surface, so a change from one step to the next shows by flipping.
 */
export function InspectionPanel({ source, steps }: Props) {
  const [index, setIndex] = useState(0);
  const [following, setFollowing] = useState(true);
  // While a run is going, show the newest step until the person picks one.
  const steps_ = steps.filter((step) => step.step !== "steps");
  useEffect(() => {
    if (following) setIndex(Math.max(0, steps_.length - 1));
  }, [steps_.length, following]);
  if (steps.length === 0) return null;
  const current = steps[Math.min(index, steps.length - 1)]!;
  const go = (next: number) => {
    setFollowing(false);
    setIndex(Math.max(0, Math.min(steps.length - 1, next)));
  };
  return (
    <section class="panel inspection" aria-labelledby="inspection-heading">
      <div class="panel-head">
        <h2 id="inspection-heading">{t("run.inspection.heading")}</h2>
        <span class="sub">
          {t("run.inspection.position", { index: index + 1, total: steps.length, label: engineText(current.label) })}
        </span>
      </div>
      <div class="inspection-steps" role="tablist" aria-label={t("run.inspection.steps")}>
        {steps.map((step, position) => (
          <button
            key={step.path}
            type="button"
            role="tab"
            class={`button small${position === index ? " primary" : ""}`}
            aria-selected={position === index}
            onClick={() => go(position)}
          >
            {engineText(step.label)}
          </button>
        ))}
      </div>
      <InspectionImage source={source} step={current} />
      <div class="actions">
        <button type="button" class="button" onClick={() => go(index - 1)} disabled={index === 0} aria-label={t("run.inspection.previousStep")}>
          {t("run.inspection.previous")}
        </button>
        <button type="button" class="button" onClick={() => go(index + 1)} disabled={index >= steps.length - 1} aria-label={t("run.inspection.nextStep")}>
          {t("run.inspection.next")}
        </button>
      </div>
    </section>
  );
}

function InspectionImage({ source, step }: { source: RunSource; step: Inspection }) {
  const { url, failed } = useImageUrl(source, step.path);
  return (
    <div class="inspection-frame">
      {url !== undefined && !failed ? <img src={url} alt={t("run.inspection.alt", { label: engineText(step.label) })} /> : <span class="sheet-placeholder">{failed ? t("run.image.unavailable") : t("run.image.loading")}</span>}
    </div>
  );
}
