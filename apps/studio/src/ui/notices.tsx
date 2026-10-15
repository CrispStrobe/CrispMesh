import { useEffect, useState } from "preact/hooks";

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
          <h1>Licenses</h1>
          <p class="sub">
            CrispMesh is free software under the GNU Affero General Public License, version 3 only, with an additional permission for the builds its author publishes in app stores (shown first below). Its source code is at{" "}
            <span class="mono wrap">https://github.com/CrispStrobe/CrispMesh</span>.
          </p>
        </div>
      </div>
      <section class="card" aria-labelledby="n-third">
        <h2 id="n-third">License, store permission and third-party software</h2>
        {missing ? (
          <p>
            This build does not carry the collected notices (they are written into release builds). The list of what the app
            contains, with each license, is in the source: <span class="mono wrap">apps/studio/docs/THIRD-PARTY-LICENSES.md</span>.
          </p>
        ) : text === null ? (
          <p role="status">Loading...</p>
        ) : (
          <pre class="notices" tabIndex={0}>
            {text}
          </pre>
        )}
      </section>
    </div>
  );
}
