import { humanise } from "../core/format";

/** Display names of the pipeline stages. Stages added later fall back to their own name. */
export const STAGE_TEXT: Record<string, { title: string; what: string }> = {
  masks: { title: "Masks", what: "The object in every photo" },
  cameras: { title: "Cameras", what: "Where each photo was taken" },
  inputs: { title: "Inputs", what: "Photos, cameras, masks" },
  stereo: { title: "Stereo", what: "Depth from photo pairs" },
  mesh: { title: "Mesh", what: "Closed surface" },
  texture: { title: "Texture", what: "Photo colours on the surface" },
  surface: { title: "Neural surface", what: "A surface trained on the photos" },
  splats: { title: "2D splats", what: "Flat splats trained on the photos" },
  check: { title: "Check", what: "Compare with the photos" },
  evaluate: { title: "Evaluate", what: "Compare with a scan" },
  run: { title: "Run", what: "" },
};

export function stageTitle(name: string): string {
  return STAGE_TEXT[name]?.title ?? humanise(name);
}
