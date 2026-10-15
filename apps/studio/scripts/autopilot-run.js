// Runs INSIDE the native app's web view (debug builds only):
//
//   CRISPMESH_STUDIO_AUTOPILOT=scripts/autopilot-run.js <the debug binary>
//
// It waits for the shell's own engine, starts a small run through the New run form
// (finding the inputs with the folder browser), waits for it to finish, checks that
// surfaces and sheets were shown and that the Content Security Policy blocked nothing,
// and reports each step on the app's stderr as "[autopilot] ...". Whoever started the
// app can then take a screenshot; the script quits the app after AUTOPILOT_LINGER ms.
//
// The scene to use is the first folder marked "Inputs" found by walking the data folder.

(async () => {
  const internals = window.__TAURI_INTERNALS__;
  const log = (line) => internals.invoke("autopilot_log", { line: String(line) });
  // Pauses are timed by the shell: a hidden web view slows or stops its own timers.
  const sleep = (ms) => internals.invoke("autopilot_sleep", { ms });
  const violations = [];
  document.addEventListener("securitypolicyviolation", (event) => {
    violations.push(`${event.violatedDirective}: ${event.blockedURI}`);
  });
  window.addEventListener("error", (event) => log(`page error: ${event.message}`));

  async function until(what, test, timeout = 60000) {
    const end = Date.now() + timeout;
    for (;;) {
      const value = test();
      if (value) return value;
      if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
      await sleep(100);
    }
  }
  const one = (selector, text) =>
    [...document.querySelectorAll(selector)].find((element) => text === undefined || element.textContent.includes(text));
  function type(selector, value) {
    const input = document.querySelector(selector);
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }

  // A wrapper may set window.AUTOPILOT before this script: { settings: {...} or null for the
  // defaults, cancel: true to cancel the run once its first surface shows, linger: ms }.
  const options = window.AUTOPILOT ?? {};
  const SMALL = {
    sizes: "64, 128", grid: "96", planes: "48", neighbours: "4", best_of: "2", vote_neighbours: "4",
    min_votes: "2, 2", crop_padding: "6", hull_dilate: "1", windows: "5, 7", aggregates: "1, 1",
  };
  const SETTINGS = options.settings === undefined ? SMALL : (options.settings ?? {});
  // The longest stretch the page could not run a timer: a measure of responsiveness.
  let stall = 0;
  let tick = performance.now();
  setInterval(() => {
    const now = performance.now();
    stall = Math.max(stall, now - tick - 100);
    tick = now;
  }, 100);
  const memory = async () => {
    const [, peak] = await internals.invoke("autopilot_memory");
    return `${Math.round(peak / 1e6)} MB peak resident`;
  };

  try {
    await log(`origin ${location.origin}, hash ${location.hash || "(none)"}`);
    await until("the runs screen of the local engine", () => location.hash.startsWith("#/engine") && one("h1", "Runs"), 90000);
    await log(`runs screen: ${one(".page-head .sub").textContent.trim()}`);

    location.hash = "#/engine/new";
    await until("the settings form", () => document.querySelector("#s-sizes"));
    // What the form sends is recorded, to compare with what was meant.
    const invoke = internals.invoke;
    internals.invoke = (command, args, ...rest) => {
      if (command === "native_start") log(`start request: ${JSON.stringify(args.body)}`);
      return invoke(command, args, ...rest);
    };
    if (options.photos) {
      // The photos start: a photos folder inside the data folder, a calibration from the list
      // the app found, and a provider for the masks and for the cameras.
      one(".segmented button", "Turntable photos").click();
      await until("the photos field", () => document.querySelector("#f-photos"));
      type("#f-photos", options.photos);
      await until("the calibrations the app found", () => document.querySelector(".calibrations button"), 15000);
      (one(".calibrations button", options.calibration ?? "") ?? document.querySelector(".calibrations button")).click();
      for (const [module, provider] of Object.entries(options.providers ?? {})) {
        const radio = document.querySelector(`input[name="provider-${module}"][value="${provider}"]`);
        if (!radio) throw new Error(`no provider ${provider} for ${module}`);
        if (radio.disabled) throw new Error(`provider ${provider} is not available: ${radio.closest("label").querySelector(".field-error")?.textContent}`);
        radio.click();
      }
      await sleep(400);
      const offered = [...document.querySelectorAll(".provider-option")].map((label) => `${label.querySelector("input").name.replace("provider-", "")}:${label.querySelector("input").value}${label.classList.contains("unavailable") ? " (not available)" : ""}${label.classList.contains("chosen") ? " [chosen]" : ""}`);
      await log(`providers: ${offered.join(", ")}`);
      await log(`photos ${document.querySelector("#f-photos").value}; calibration ${document.querySelector("#f-calibration").value.split("/").pop()}`);
    } else {
      one(".path-row button", "Browse").click();
      // Walk down until a folder marked as inputs shows up.
      for (let depth = 0; depth < 4; depth++) {
        await until("a folder listing", () => document.querySelector("#f-inputs-browser .browser-list"));
        const use = document.querySelector("#f-inputs-browser li.pickable .button");
        if (use) {
          use.click();
          await sleep(300);
          break;
        }
        const folder = document.querySelector("#f-inputs-browser .browser-entry");
        if (!folder) throw new Error("the data folder has no inputs folder in it");
        folder.click();
        await sleep(400);
      }
      const inputs = document.querySelector("#f-inputs").value;
      await log(`inputs chosen with the folder browser: ${inputs}`);
      if (!inputs) throw new Error("no inputs were chosen");
    }
    for (const [name, value] of Object.entries(SETTINGS)) type(`#s-${name}`, value);
    type("#f-name", options.name ?? "native app check");
    const device = document.querySelector("#f-device");
    if (device) {
      device.value = "cpu";
      device.dispatchEvent(new Event("change", { bubbles: true }));
    }
    // Let the form re-render: the submit handler must see what was just typed.
    await sleep(500);
    const changed = one(".settings-head .sub").textContent.trim();
    await log(`form: ${changed}, device ${device ? device.value : "chosen by the engine"}`);
    const expected = Object.keys(SETTINGS).length;
    if (expected > 0 && !changed.startsWith(String(expected))) throw new Error("the settings did not take");
    one("button[type=submit]", "Start run").click();

    await until("the run view", () => location.hash.startsWith("#/engine/run/"), 30000);
    await log(`run started: ${decodeURIComponent(location.hash.split("/").pop())}`);
    let lastSurface = "";
    let cancelled = false;
    let firstSurface = true;
    let worst = 0;
    let sheets = 0;
    const began = Date.now();
    const seconds = () => `${Math.round((Date.now() - began) / 1000)} s`;
    await until(
      "the run to finish",
      () => {
        const surface = document.querySelector("#mesh-heading + .sub")?.textContent ?? "";
        if (surface && surface !== lastSurface) {
          lastSurface = surface;
          log(`surface on screen: ${surface} (${seconds()}; longest page stall since the last line ${Math.round(stall)} ms)`);
          worst = Math.max(worst, firstSurface ? 0 : stall);
          firstSurface = false;
          stall = 0;
          if (options.cancel && !cancelled && surface.includes("triangles")) {
            cancelled = true;
            one("button", "Cancel run")?.click();
            log("cancel requested with the Cancel run button");
          }
        }
        const shown = document.querySelectorAll(".sheet").length;
        if (shown !== sheets) {
          sheets = shown;
          log(`sheets on screen: ${shown} (${seconds()})`);
        }
        const badge = document.querySelector(".run-meta .badge")?.textContent;
        return badge && badge !== "Running" && badge !== "Waiting";
      },
      Number(options.timeout ?? 900000),
    );
    const status = document.querySelector(".run-meta .badge").textContent;
    await log(`run ${status} after ${seconds()}; ${await memory()}; longest page stall while it ran ${Math.round(Math.max(worst, stall))} ms (after the first surface)`);
    if (options.cancel) {
      await sleep(500);
      await log(`notice: ${one(".notice", "cancelled")?.textContent.trim().slice(0, 80)}`);
      await log(`CSP violations: ${violations.length === 0 ? "none" : violations.join(" | ")}`);
      await log(status === "Cancelled" ? "AUTOPILOT DONE ok" : "AUTOPILOT DONE with problems");
      await internals.invoke("autopilot_quit", { afterMs: Number(options.linger ?? 6000) });
      return;
    }
    const finalButton = one("button", "Load final");
    if (finalButton) {
      await log(`final mesh waits for its button: ${finalButton.textContent.trim()}`);
      finalButton.click();
    }
    try {
      await until("the final surface", () => (document.querySelector("#mesh-heading + .sub")?.textContent ?? "").includes("Final surface"), 180000);
    } catch (problem) {
      // Say what the panel shows instead, so a failure can be understood from the log alone.
      const panel = document.querySelector(".mesh-panel");
      await log(`surface panel: ${(panel?.textContent ?? "(none)").replace(/\s+/g, " ").slice(0, 400)}`);
      throw problem;
    }
    if (options.up !== undefined) {
      const up = document.querySelector('select[aria-label="Up direction of the model"]');
      if (!up || ![...up.options].some((choice) => choice.value === options.up)) throw new Error(`no up direction ${options.up}`);
      up.value = options.up;
      up.dispatchEvent(new Event("change", { bubbles: true }));
      await log(`viewer up direction: ${options.up}`);
    }
    await sleep(1500);
    // Whether the surface is drawn at all: a picture of an empty view would still pass a blank check.
    const probe = window.__crispmeshProbe?.();
    await log(`3D view: ${probe ? `${probe.surface} of ${probe.samples} sampled pixels show the surface` : "no probe"}`);
    const drawn = probe !== undefined && probe.surface > 0;
    const images = [...document.querySelectorAll(".sheet img")];
    const loaded = images.filter((image) => image.naturalWidth > 0).length;
    const reports = [...document.querySelectorAll(".report h3")].map((heading) => heading.textContent);
    const canvas = document.querySelector(".viewer canvas");
    await log(`metrics: ${[...document.querySelectorAll(".metrics > div")].map((row) => row.querySelector("dt").childNodes[0].textContent + "=" + row.querySelector("dd").textContent).join(", ")}`);
    await log(`sheet labels: ${[...document.querySelectorAll(".sheet-label")].map((label) => label.textContent).join(" | ")}`);
    await log(`run ${status}; stages: ${[...document.querySelectorAll(".stage")].map((s) => s.querySelector(".stage-name").textContent + "=" + s.querySelector(".stage-state").textContent.trim()).join(", ")}`);
    await log(`surface: ${document.querySelector("#mesh-heading + .sub").textContent}; canvas ${canvas.width}x${canvas.height}`);
    await log(`sheets loaded: ${loaded} of ${document.querySelectorAll(".sheet").length}; reports: ${reports.join(", ")}`);
    await log(`CSP violations: ${violations.length === 0 ? "none" : violations.join(" | ")}`);
    const ok = status === "Complete" && loaded > 0 && loaded === images.length && violations.length === 0 && drawn;
    if (!drawn) await log("the 3D view draws nothing");
    // With `marks`, the screens a person sees are shown one after another, each announced as
    // "MARK <name>" and held, so whoever started the app can photograph them (CI screenshots).
    if (options.marks) {
      const hold = Number(options.hold ?? 5000);
      // Scrolls so that `element` is just below the sticky top bar, whichever element scrolls.
      const show = (element) => {
        if (!element) return;
        element.scrollIntoView({ block: "start" });
        const bar = document.querySelector(".topbar")?.getBoundingClientRect().bottom ?? 0;
        window.scrollBy(0, element.getBoundingClientRect().top - bar - 12);
      };
      // A window capture on a virtual GPU (CI runners) can miss the WebGL layer although the
      // frame was drawn (the probe reads it back). The frame the GPU drew is laid over the
      // canvas as a picture, so the screenshot shows it. Debug builds only, like this script.
      const still = () => {
        const canvas = document.querySelector(".viewer canvas");
        const shot = window.__crispmeshProbe?.();
        if (!canvas || !shot?.picture || shot.surface === 0) return;
        let image = document.querySelector("img.autopilot-still");
        if (!image) {
          image = document.createElement("img");
          image.className = "autopilot-still";
          image.alt = "";
          image.style.cssText = "position:absolute;inset:0;width:100%;height:100%;pointer-events:none";
          // Right after the canvas, so the label of the surface stays on top of it.
          canvas.after(image);
        }
        image.src = shot.picture;
      };
      // Held until the driver says it has the picture (it creates ack-<name>), or `hold` passes.
      const mark = async (name) => {
        await sleep(1200);
        await log(`MARK ${name}`);
        const acknowledged = await internals.invoke("autopilot_await", { name, limitMs: Number(options.ackLimit ?? 120000) }).catch(() => false);
        if (!acknowledged) await sleep(hold);
      };
      (document.scrollingElement ?? document.documentElement).scrollTop = 0;
      still();
      await mark("run");
      show(document.querySelector(".mesh-panel"));
      still();
      await mark("surface");
      show(document.querySelector(".gallery"));
      await mark("sheets");
      location.hash = "#/engine";
      await until("the runs list", () => document.querySelector(".run-row"));
      (document.scrollingElement ?? document.documentElement).scrollTop = 0;
      await mark("runs");
      location.hash = "#/engine/new";
      await until("the new run form", () => document.querySelector("#s-sizes"));
      one(".segmented button", "Turntable photos")?.click();
      await sleep(600);
      show(document.querySelector("#f-photos")?.closest("fieldset") ?? document.querySelector("form"));
      await mark("new-run");
    }
    await log(ok ? "AUTOPILOT DONE ok" : "AUTOPILOT DONE with problems");
  } catch (problem) {
    await log(`CSP violations: ${violations.length === 0 ? "none" : violations.join(" | ")}`);
    await log(`AUTOPILOT FAILED: ${problem.message}`);
  }
  const linger = Number(options.linger ?? 15000);
  await internals.invoke("autopilot_quit", { afterMs: linger });
})();
