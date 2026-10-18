import { describe, expect, it } from "vitest";
import { parseRoute } from "../ui/app";
import { cameraFieldOfView, getShell, sameConfig, sourceText, type ShellConfig } from "./shell";

const config: ShellConfig = {
  repo: "/r",
  python: "",
  torch_python: "",
  runs_dir: "",
  data_dir: "",
  device: "auto",
  alicevision: "",
  alicevision_library_path: "",
  colmap: "",
  sam_python: "",
  sam_source: "",
  sam_checkpoint: "",
};

describe("the native shell, seen from the web side", () => {
  it("is absent in a browser (and in these tests)", () => {
    expect(getShell()).toBeNull();
  });

  it("knows no camera field of view in a browser", async () => {
    expect(await cameraFieldOfView(1920, 1080)).toBeNull();
  });

  it("words where a value comes from", () => {
    expect(sourceText("environment", "repo")).toBe("from CRISPMESH_REPO");
    expect(sourceText("environment", "torch_python")).toBe("from CRISPMESH_TORCH_PYTHON");
    expect(sourceText("environment", "data_dir")).toBe("from CRISPMESH_DATA_DIR");
    expect(sourceText("environment", "device")).toBe("from the environment");
    expect(sourceText("found", "repo")).toBe("found automatically");
    expect(sourceText("default", "torch_python")).toBe("same as the interpreter above");
    expect(sourceText("default", "runs_dir")).toBe("default");
  });

  it("knows when there is something to save", () => {
    expect(sameConfig(config, { ...config })).toBe(true);
    expect(sameConfig(config, { ...config, repo: " /r " })).toBe(true);
    expect(sameConfig(config, { ...config, device: "cpu" })).toBe(false);
  });

  it("has a route for the engine settings", () => {
    expect(parseRoute("#/shell")).toEqual({ screen: "shell" });
    expect(parseRoute("#/engine/run/20261005-1%20x")).toEqual({ screen: "run", id: "20261005-1 x" });
    expect(parseRoute("")).toEqual({ screen: "connection" });
    expect(parseRoute("#/replay?bundle=big%2F")).toEqual({ screen: "replay", bundle: "big/" });
  });
});
