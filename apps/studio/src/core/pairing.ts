/**
 * Pairing a phone with a computer that lets other devices use its engine: the computer shows a QR code with its
 * address and token, the phone scans it. The text is a small JSON object that only this app reads.
 */

export interface Pairing {
  url: string;
  token: string;
}

/** The text of the code: `{"crispmesh_engine": "http://192.168.1.20:8766", "token": "..."}`. */
export function pairingText(pairing: Pairing): string {
  return JSON.stringify({ crispmesh_engine: pairing.url, token: pairing.token });
}

/** The address and token in a scanned code, or null when the code is something else. */
export function parsePairing(text: string): Pairing | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  const record = value as { crispmesh_engine?: unknown; token?: unknown } | null;
  if (typeof record?.crispmesh_engine !== "string" || typeof record.token !== "string") return null;
  if (!/^https?:\/\/[^\s/]+(:\d+)?\/?$/.test(record.crispmesh_engine)) return null;
  if (!/^[0-9a-f]{16,128}$/i.test(record.token)) return null;
  return { url: record.crispmesh_engine, token: record.token };
}

/**
 * Pairing with the six-digit code shown on the computer (for a computer found on the network instead of scanned):
 * `POST /api/pair` answers the token for the right code. `fetcher`: how requests reach the computer (in the apps,
 * through the app). Throws with the engine's message for a wrong code.
 */
export async function pairWithCode(
  fetcher: (input: string, init?: RequestInit) => Promise<Response>,
  url: string,
  code: string,
): Promise<Pairing> {
  const digits = code.replace(/\s+/g, "");
  if (!/^\d{6}$/.test(digits)) throw new Error("The pairing code has six digits (shown on the computer).");
  const address = url.replace(/\/+$/, "");
  const response = await fetcher(`${address}/api/pair`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ code: digits }),
  });
  const answer = (await response.json().catch(() => ({}))) as { token?: unknown; error?: unknown };
  if (!response.ok || typeof answer.token !== "string") {
    throw new Error(typeof answer.error === "string" ? answer.error : `The computer answered ${response.status}.`);
  }
  return { url: address, token: answer.token };
}
