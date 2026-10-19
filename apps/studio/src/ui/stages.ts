import { humanise } from "../core/format";
import { t, type MessageKey } from "../i18n";

/** Pipeline stages with a translated name and a one-line description (`what`; none for `run`). */
const STAGE_KEYS: Record<string, { title: MessageKey; what?: MessageKey }> = {
  masks: { title: "run.stage.masks.title", what: "run.stage.masks.what" },
  cameras: { title: "run.stage.cameras.title", what: "run.stage.cameras.what" },
  inputs: { title: "run.stage.inputs.title", what: "run.stage.inputs.what" },
  hull: { title: "run.stage.hull.title", what: "run.stage.hull.what" },
  stereo: { title: "run.stage.stereo.title", what: "run.stage.stereo.what" },
  mesh: { title: "run.stage.mesh.title", what: "run.stage.mesh.what" },
  texture: { title: "run.stage.texture.title", what: "run.stage.texture.what" },
  surface: { title: "run.stage.surface.title", what: "run.stage.surface.what" },
  splats: { title: "run.stage.splats.title", what: "run.stage.splats.what" },
  check: { title: "run.stage.check.title", what: "run.stage.check.what" },
  evaluate: { title: "run.stage.evaluate.title", what: "run.stage.evaluate.what" },
  run: { title: "run.stage.run.title" },
};

/**
 * Display names of the pipeline stages in the active language (read when accessed, so a
 * language change shows on the next render). Stages added later fall back to their own name.
 */
export const STAGE_TEXT: Record<string, { readonly title: string; readonly what: string }> = Object.fromEntries(
  Object.entries(STAGE_KEYS).map(([name, keys]) => [
    name,
    {
      get title() {
        return t(keys.title);
      },
      get what() {
        return keys.what === undefined ? "" : t(keys.what);
      },
    },
  ]),
);

export function stageTitle(name: string): string {
  return STAGE_TEXT[name]?.title ?? humanise(name);
}
