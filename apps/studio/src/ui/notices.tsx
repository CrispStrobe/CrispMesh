import { useEffect, useState } from "preact/hooks";
import { t, tParts } from "../i18n";

/** The file that `npm run build:release` writes next to index.html. */
export const NOTICES_FILE = "THIRD-PARTY-NOTICES.txt";

/**
 * Licenses of the app and of everything in it, readable inside the app: a store build has
 * no browser tab to open the file in, and a phone has no folder to look for it.
 */
export function Notices() {
  const [text, setText] = useState<string | null>(null);
  const [missing, setMissing] = useState(false);
  useEffect(() => {
    let alive = true;
    fetch(NOTICES_FILE)
      .then((response) => (response.ok ? response.text() : Promise.reject(new Error(String(response.status)))))
      .then(
        (body) => alive && setText(body),
        () => alive && setMissing(true),
      );
    return () => {
      alive = false;
    };
  }, []);
  return (
    <div class="page narrow">
      <div class="page-head">
        <div>
          <h1>{t("app.notices.title")}</h1>
          <p class="sub">{tParts("app.notices.intro", { url: <span class="mono wrap">https://github.com/CrispStrobe/CrispMesh</span> })}</p>
        </div>
      </div>
      <section class="card" aria-labelledby="n-third">
        <h2 id="n-third">{t("app.notices.heading")}</h2>
        {missing ? (
          <p>{tParts("app.notices.missing", { path: <span class="mono wrap">apps/studio/docs/THIRD-PARTY-LICENSES.md</span> })}</p>
        ) : text === null ? (
          <p role="status">{t("app.notices.loading")}</p>
        ) : (
          <pre class="notices" tabIndex={0}>
            {text}
          </pre>
        )}
      </section>
    </div>
  );
}
