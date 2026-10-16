import { useEffect, useState } from "preact/hooks";
import { getShell } from "../shell/shell";
import { describe } from "../sources/transport";

interface Sharing {
  running: boolean;
  port?: number;
  token?: string;
  addresses?: string[];
}

/**
 * "Let other devices use this computer" (desktop app): serves the built-in engine on the local network, so a phone
 * that took the photos can send them here, start the run here and follow it. Shows the address and the token to type
 * on the phone (Connection, "Another engine").
 */
export function ShareEngine() {
  const shell = getShell();
  const [sharing, setSharing] = useState<Sharing | null>(null);
  const [mobile, setMobile] = useState(true);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (shell === null) return;
    void shell.info().then((info) => setMobile(info.os === "ios" || info.os === "android"));
    void shell.bridge<Sharing>("share_engine", {}).then(setSharing, () => setSharing({ running: false }));
  }, []);

  if (shell === null || mobile || sharing === null) return null;
  const turn = (on: boolean) => {
    setBusy(true);
    setProblem("");
    shell
      .bridge<Sharing>("share_engine", { on })
      .then(setSharing, (error) => setProblem(describe(error)))
      .finally(() => setBusy(false));
  };
  const copy = (text: string) => void navigator.clipboard?.writeText(text).catch(() => undefined);

  return (
    <div class="share-engine">
      <label class="check">
        <input type="checkbox" checked={sharing.running} disabled={busy} onChange={(event) => turn(event.currentTarget.checked)} />
        <span>Let other devices use this computer</span>
      </label>
      <span class="help">
        A phone or tablet takes the photos and sends them here; the run is computed on this computer and followed on the
        phone. On the phone: Connection, Another engine, with this address and token. Local network only, no encryption:
        use it on a network you trust.
      </span>
      {sharing.running && (
        <dl class="share-details">
          <dt>Address</dt>
          <dd>
            {(sharing.addresses ?? []).length === 0 ? (
              <span>this computer's address on the network, port {sharing.port}</span>
            ) : (
              (sharing.addresses ?? []).map((address) => (
                <span key={address} class="mono">
                  {address}{" "}
                  <button type="button" class="button small" onClick={() => copy(address)}>
                    Copy
                  </button>
                </span>
              ))
            )}
          </dd>
          <dt>Token</dt>
          <dd>
            <span class="mono">{sharing.token}</span>{" "}
            <button type="button" class="button small" onClick={() => copy(sharing.token ?? "")}>
              Copy
            </button>
          </dd>
        </dl>
      )}
      {problem !== "" && (
        <p class="field-error" role="alert">
          {problem}
        </p>
      )}
    </div>
  );
}
