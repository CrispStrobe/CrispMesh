import jsQR from "jsqr";
import qrcode from "qrcode-generator";
import { describe, expect, it } from "vitest";
import { pairingText, parsePairing } from "./pairing";

describe("pairing a phone with a computer", () => {
  it("reads back what the computer shows", () => {
    const pairing = { url: "http://192.168.1.20:8766", token: "0123456789abcdef0123456789abcdef" };
    expect(parsePairing(pairingText(pairing))).toEqual(pairing);
  });

  it("ignores other codes and anything that is not a plain address and token", () => {
    expect(parsePairing("https://example.org")).toBeNull();
    expect(parsePairing('{"crispmesh_engine": "javascript:alert(1)", "token": "0123456789abcdef"}')).toBeNull();
    expect(parsePairing('{"crispmesh_engine": "http://host:8766/path", "token": "0123456789abcdef"}')).toBeNull();
    expect(parsePairing('{"crispmesh_engine": "http://host:8766", "token": "short"}')).toBeNull();
    expect(parsePairing('{"crispmesh_engine": "http://host:8766", "token": "not hex not hex not hex"}')).toBeNull();
  });

  it("comes back through a picture of the code (as the phone's camera sees it, here sharp)", () => {
    const pairing = { url: "http://192.168.178.42:8766", token: "a3f9c0e1b2d4f6a8c0e2b4d6f8a0c2e4" };
    const qr = qrcode(0, "M");
    qr.addData(pairingText(pairing));
    qr.make();
    const cells = qr.getModuleCount();
    const [cell, margin] = [4, 4];
    const size = (cells + 2 * margin) * cell;
    const rgba = new Uint8ClampedArray(size * size * 4).fill(255);
    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        const [row, column] = [Math.floor(y / cell) - margin, Math.floor(x / cell) - margin];
        if (row >= 0 && column >= 0 && row < cells && column < cells && qr.isDark(row, column)) {
          rgba.fill(0, (y * size + x) * 4, (y * size + x) * 4 + 3);
        }
      }
    }
    const found = jsQR(rgba, size, size);
    expect(found).not.toBeNull();
    expect(parsePairing(found!.data)).toEqual(pairing);
  });
});
