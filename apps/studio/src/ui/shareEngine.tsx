import qrcode from "qrcode-generator";
import { useEffect, useMemo, useState } from "preact/hooks";
import { pairingText } from "../core/pairing";
import { getShell } from "../shell/shell";
import { describe } from "../sources/transport";

interface Sharing {
  running: boolean;
  port?: number;
  token?: string;
  addresses?: string[];
  /** Six digits for a phone that found this computer on the network (replaced after five wrong tries). */
  code?: string;
  /** Announced on the network (Bonjour), so phones can find it. */
  announced?: boolean;
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

  // While sharing: read the state again now and then (the pairing code changes after wrong tries).
  useEffect(() => {
    if (shell === null || sharing?.running !== true) return;
    const timer = setInterval(() => {
      void shell.bridge<Sharing>("share_engine", {}).then(setSharing, () => undefined);
    }, 3000);
    return () => clearInterval(timer);
  }, [sharing?.running]);

  // The pairing code for the phone: the first address and the token.
  const code = useMemo(() => {
    const address = sharing?.addresses?.[0];
    if (!sharing?.running || address === undefined || sharing.token === undefined) return "";
    const qr = qrcode(0, "M");
    qr.addData(pairingText({ url: address, token: sharing.token }));
    qr.make();
    return qr.createDataURL(5, 4);
  }, [sharing?.running, sharing?.token, sharing?.addresses?.[0]]);

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
        phone. On the phone: Connection, Another engine, then scan this computer's code, or find this computer on the
        network and type its pairing code. Local network only, no encryption: use it on a network you trust.
      </span>
      {code !== "" && (
        <figure class="share-code">
          <img src={code} alt="Pairing code: address and token of this computer's engine" />
          <figcaption class="help">On the phone: Connection, Another engine, Scan the computer's code.</figcaption>
        </figure>
      )}
      {sharing.running && (
        <dl class="share-details">
          {sharing.code !== undefined && sharing.code !== "" && (
            <>
              <dt>Pairing code</dt>
              <dd>
                <span class="mono pairing-code">{sharing.code}</span>
                <span class="help">
                  {sharing.announced === true
                    ? " For a phone that found this computer on the network."
                    : " This computer could not announce itself on the network: scan the code instead."}
                </span>
              </dd>
            </>
          )}
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
