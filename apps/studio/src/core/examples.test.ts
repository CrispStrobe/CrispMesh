import { describe, expect, it } from "vitest";
import { downloadExample, parseExampleManifest, runAttribution, safePath, sha256, type ExampleFile, type ExampleObject, type ExampleStore } from "./examples";

const bytes = (text: string) => new TextEncoder().encode(text).buffer as ArrayBuffer;

async function manifestWith(contents: Record<string, string>) {
  const entry = async (path: string) => ({ path, size: contents[path]!.length, sha256: await sha256(bytes(contents[path]!)) });
  return {
    schema: "crispmesh_example_objects_v1",
    source: "https://data.mendeley.com/datasets/ngvgpsvd8b/1",
    doi: "10.17632/ngvgpsvd8b.1",
    license: "CC-BY-4.0",
    attribution: "3DLF-Scan v1 by Bohdan Vodianyk, Enrique Nava-Baro and Anton Popov, DOI 10.17632/ngvgpsvd8b.1, licensed CC BY 4.0.",
    calibration_files: [await entry("calibration/3dlf-pro.json")],
    objects: [
      {
        name: "dragon",
        title: "Dragon",
        photos: 3,
        bytes: 0,
        calibration: "calibration/3dlf-pro.json",
        files: await Promise.all(["dragon/rgb/dragon_10_rgb.png", "dragon/rgb/dragon_2_rgb.png", "dragon/rgb/dragon_0_rgb.png"].map(entry)),
      },
      { name: "../evil", title: "Evil", calibration: "calibration/3dlf-pro.json", files: [] },
      { name: "nocal", title: "No calibration", calibration: "calibration/other.json", files: await Promise.all(["dragon/rgb/dragon_0_rgb.png", "dragon/rgb/dragon_2_rgb.png", "dragon/rgb/dragon_10_rgb.png"].map(entry)) },
    ],
  };
}

const contents: Record<string, string> = {
  "calibration/3dlf-pro.json": '{"schema":"crispmesh_lens_calibration_v1"}',
  "dragon/rgb/dragon_0_rgb.png": "photo 0",
  "dragon/rgb/dragon_2_rgb.png": "photo 2",
  "dragon/rgb/dragon_10_rgb.png": "photo 10",
};

class MemoryStore implements ExampleStore {
  files = new Map<string, string>();
  freeBytes: number | null = 10 ** 10;
  have(): Promise<Set<string>> {
    return Promise.resolve(new Set(this.files.keys()));
  }
  put(_object: ExampleObject, file: ExampleFile, data: ArrayBuffer): Promise<void> {
    this.files.set(file.path, new TextDecoder().decode(data));
    return Promise.resolve();
  }
  free(): Promise<number | null> {
    return Promise.resolve(this.freeBytes);
  }
  remove(): Promise<void> {
    this.files.clear();
    return Promise.resolve();
  }
}

describe("example objects", () => {
  it("reads the dataset's manifest: photos in capture order, the calibration file, only safe objects", async () => {
    const manifest = parseExampleManifest(await manifestWith(contents), "https://huggingface.co/datasets/cstr/3dlf-scan-photos/resolve/main/manifest.json");
    expect(manifest.baseUrl).toBe("https://huggingface.co/datasets/cstr/3dlf-scan-photos/resolve/main/");
    expect(manifest.objects.map((object) => object.id)).toEqual(["dragon"]);
    const dragon = manifest.objects[0]!;
    expect(dragon.name).toBe("Dragon");
    expect(dragon.files.map((file) => file.path.split("/").pop())).toEqual(["dragon_0_rgb.png", "dragon_2_rgb.png", "dragon_10_rgb.png"]);
    expect(dragon.calibration.path).toBe("calibration/3dlf-pro.json");
    expect(dragon.bytes).toBe(Object.values(contents).reduce((sum, text) => sum + text.length, 0));
    expect(runAttribution(manifest, dragon)).toContain("DOI 10.17632/ngvgpsvd8b.1");
    expect(runAttribution(manifest, dragon)).not.toContain("Stanford");
    const withPurpose = parseExampleManifest(await manifestWith(contents), "https://huggingface.co/datasets/cstr/3dlf-scan-photos/resolve/main/manifest.json", "Prints of Stanford 3D Scanning Repository models.");
    expect(runAttribution(withPurpose, withPurpose.objects[0]!)).toContain("Stanford 3D Scanning Repository");
    expect(() => parseExampleManifest({ schema: "other" }, "https://x/")).toThrow(/not a list/);
    const raw = await manifestWith(contents);
    expect(() => parseExampleManifest({ ...raw, attribution: undefined }, "https://x/")).toThrow(/where the photos come from/);
    expect(() => parseExampleManifest(raw, "http://example.org/m.json")).toThrow(/HTTPS/);
    for (const bad of ["/etc/passwd.png", "a/../b.png", "a\\b.png", ".hidden/a.png", "a/b.exe"]) expect(safePath(bad)).toBe(false);
  });

  it("downloads what is missing, checks every file's size and SHA-256, cancels, and refuses when space is short", async () => {
    const manifest = parseExampleManifest(await manifestWith(contents), "https://hf.example/datasets/x/resolve/main/manifest.json");
    const dragon = manifest.objects[0]!;
    const store = new MemoryStore();
    const asked: string[] = [];
    const fetcher = async (url: string) => {
      asked.push(url);
      const path = url.replace(manifest.baseUrl, "");
      return new Response(contents[path] ?? "", { status: contents[path] === undefined ? 404 : 200 });
    };
    store.files.set("dragon/rgb/dragon_0_rgb.png", "photo 0");
    const progress: number[] = [];
    await downloadExample(manifest, dragon, store, { fetcher, onProgress: (p) => progress.push(p.done) });
    expect(asked.map((url) => url.replace(manifest.baseUrl, ""))).toEqual(["calibration/3dlf-pro.json", "dragon/rgb/dragon_2_rgb.png", "dragon/rgb/dragon_10_rgb.png"]);
    expect(progress).toEqual([1, 2, 3, 4]);
    expect(store.files.size).toBe(4);

    const tampered = new MemoryStore();
    await expect(downloadExample(manifest, dragon, tampered, { fetcher: async () => new Response("photo X") })).rejects.toThrow(/SHA-256 differs|bytes arrived/);
    const controller = new AbortController();
    controller.abort();
    await expect(downloadExample(manifest, dragon, new MemoryStore(), { fetcher, signal: controller.signal })).rejects.toThrow();
    const full = new MemoryStore();
    full.freeBytes = 10;
    await expect(downloadExample(manifest, dragon, full, { fetcher })).rejects.toThrow(/Not enough free space/);
  });
});
