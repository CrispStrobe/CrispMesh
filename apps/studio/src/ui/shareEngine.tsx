import qrcode from "qrcode-generator";
import { useEffect, useMemo, useState } from "preact/hooks";
import { pairingText } from "../core/pairing";
import { getShell } from "../shell/shell";
import { describe } from "../sources/transport";
import { t } from "../i18n";

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
        <span>{t("connection.share.toggle")}</span>
      </label>
      <span class="help">{t("connection.share.help")}</span>
      {code !== "" && (
        <figure class="share-code">
          <img src={code} alt={t("connection.share.codeAlt")} />
          <figcaption class="help">{t("connection.share.codeCaption")}</figcaption>
        </figure>
      )}
      {sharing.running && (
        <dl class="share-details">
          {sharing.code !== undefined && sharing.code !== "" && (
            <>
              <dt>{t("connection.share.pairingCode")}</dt>
              <dd>
                <span class="mono pairing-code">{sharing.code}</span>
                <span class="help">
                  {sharing.announced === true
                    ? ` ${t("connection.share.announced")}`
                    : ` ${t("connection.share.notAnnounced")}`}
                </span>
              </dd>
            </>
          )}
          <dt>{t("connection.share.address")}</dt>
          <dd>
            {(sharing.addresses ?? []).length === 0 ? (
              <span>{t("connection.share.addressUnknown", { port: sharing.port ?? "" })}</span>
            ) : (
              (sharing.addresses ?? []).map((address) => (
                <span key={address} class="mono">
                  {address}{" "}
                  <button type="button" class="button small" onClick={() => copy(address)}>
                    {t("connection.share.copy")}
                  </button>
                </span>
              ))
            )}
          </dd>
          <dt>{t("connection.share.token")}</dt>
          <dd>
            <span class="mono">{sharing.token}</span>{" "}
            <button type="button" class="button small" onClick={() => copy(sharing.token ?? "")}>
              {t("connection.share.copy")}
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
