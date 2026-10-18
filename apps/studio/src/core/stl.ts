/**
 * Binary STL -> indexed triangle mesh with smooth normals and a bounding box.
 *
 * STL repeats every vertex per triangle. Welding identical vertices (exact bit equality,
 * which is what marching cubes output has) cuts memory to about a third and gives smooth
 * normals for free; flat shading is done by the material, so one buffer serves both.
 *
 * The work is resumable in slices (`StlBuilder.step`) so it can run in a worker in one go
 * or on the main thread in chunks without freezing the page. No DOM, no three.js.
 */

import { t } from "../i18n";

export interface ParsedMesh {
  /** xyz per vertex. */
  positions: Float32Array;
  /** Unit normals per vertex, area-weighted over the adjoining triangles. */
  normals: Float32Array;
  /** Three vertex indices per triangle. */
  indices: Uint32Array;
  /** Triangles in the file. */
  triangles: number;
  /** Triangles dropped because they were degenerate or not finite. */
  dropped: number;
  min: [number, number, number];
  max: [number, number, number];
}

export class StlError extends Error {}

const HEADER = 84;
const RECORD = 50;

/** Triangle count of a binary STL, after checking that the buffer can be one. */
export function stlTriangleCount(buffer: ArrayBuffer): number {
  if (buffer.byteLength < HEADER) throw new StlError(t("inputs.stl.short"));
  const view = new DataView(buffer);
  const count = view.getUint32(80, true);
  if (HEADER + RECORD * count > buffer.byteLength) {
    const head = new TextDecoder().decode(new Uint8Array(buffer, 0, 5));
    if (head === "solid") throw new StlError(t("inputs.stl.ascii"));
    throw new StlError(
      t("inputs.stl.incomplete", { count, held: Math.floor((buffer.byteLength - HEADER) / RECORD) }),
    );
  }
  return count;
}

export class StlBuilder {
  readonly triangles: number;
  private readonly view: DataView;
  private positions: Float32Array;
  private bits: Uint32Array;
  private readonly indices: Uint32Array;
  private table: Int32Array;
  private mask: number;
  private vertices = 0;
  private used = 0;
  private dropped = 0;
  private done = 0;
  private readonly min: [number, number, number] = [Infinity, Infinity, Infinity];
  private readonly max: [number, number, number] = [-Infinity, -Infinity, -Infinity];

  constructor(buffer: ArrayBuffer) {
    this.triangles = stlTriangleCount(buffer);
    this.view = new DataView(buffer);
    // A closed surface has about half as many vertices as triangles; grow if the file is a soup.
    const capacity = Math.max(64, Math.ceil(this.triangles * 0.6));
    this.positions = new Float32Array(capacity * 3);
    this.bits = new Uint32Array(this.positions.buffer);
    this.indices = new Uint32Array(this.triangles * 3);
    let size = 256;
    while (size < capacity * 2) size *= 2;
    this.table = new Int32Array(size).fill(-1);
    this.mask = size - 1;
  }

  /** Triangles processed so far. */
  get progress(): number {
    return this.done;
  }

  /** Processes up to `count` more triangles; returns true when the file is exhausted. */
  step(count: number): boolean {
    const end = Math.min(this.triangles, this.done + count);
    const view = this.view;
    const corner = [0, 0, 0];
    for (let triangle = this.done; triangle < end; triangle++) {
      const base = HEADER + triangle * RECORD + 12;
      let finite = true;
      for (let c = 0; c < 3; c++) {
        const at = base + c * 12;
        let x = view.getUint32(at, true);
        let y = view.getUint32(at + 4, true);
        let z = view.getUint32(at + 8, true);
        // Exponent all ones: infinity or NaN.
        if ((x & 0x7f800000) === 0x7f800000 || (y & 0x7f800000) === 0x7f800000 || (z & 0x7f800000) === 0x7f800000) {
          finite = false;
          break;
        }
        // -0 and +0 are the same point.
        if (x === 0x80000000) x = 0;
        if (y === 0x80000000) y = 0;
        if (z === 0x80000000) z = 0;
        corner[c] = this.vertex(x, y, z);
      }
      const a = corner[0]!;
      const b = corner[1]!;
      const c = corner[2]!;
      if (!finite || a === b || b === c || a === c) {
        this.dropped += 1;
        continue;
      }
      this.indices[this.used++] = a;
      this.indices[this.used++] = b;
      this.indices[this.used++] = c;
    }
    this.done = end;
    return end >= this.triangles;
  }

  private vertex(x: number, y: number, z: number): number {
    let slot = (Math.imul(x, 0x9e3779b1) ^ Math.imul(y, 0x85ebca6b) ^ Math.imul(z, 0xc2b2ae35)) & this.mask;
    const bits = this.bits;
    const table = this.table;
    for (;;) {
      const found = table[slot]!;
      if (found < 0) break;
      const at = found * 3;
      if (bits[at] === x && bits[at + 1] === y && bits[at + 2] === z) return found;
      slot = (slot + 1) & this.mask;
    }
    const index = this.vertices++;
    if (index * 3 + 3 > this.positions.length) this.growPositions();
    const at = index * 3;
    this.bits[at] = x;
    this.bits[at + 1] = y;
    this.bits[at + 2] = z;
    table[slot] = index;
    if (this.vertices * 2 > table.length) this.growTable();
    for (let axis = 0; axis < 3; axis++) {
      const value = this.positions[at + axis]!;
      if (value < this.min[axis]!) this.min[axis] = value;
      if (value > this.max[axis]!) this.max[axis] = value;
    }
    return index;
  }

  private growPositions(): void {
    const grown = new Float32Array(this.positions.length * 2);
    grown.set(this.positions);
    this.positions = grown;
    this.bits = new Uint32Array(grown.buffer);
  }

  private growTable(): void {
    const size = this.table.length * 2;
    const table = new Int32Array(size).fill(-1);
    const mask = size - 1;
    const bits = this.bits;
    for (let index = 0; index < this.vertices; index++) {
      const at = index * 3;
      let slot =
        (Math.imul(bits[at]!, 0x9e3779b1) ^ Math.imul(bits[at + 1]!, 0x85ebca6b) ^ Math.imul(bits[at + 2]!, 0xc2b2ae35)) & mask;
      while (table[slot]! >= 0) slot = (slot + 1) & mask;
      table[slot] = index;
    }
    this.table = table;
    this.mask = mask;
  }

  /** Call once `step` has returned true. */
  finish(): ParsedMesh {
    if (this.done < this.triangles) throw new StlError("finish() called before the file was read to the end.");
    const positions = this.positions.slice(0, this.vertices * 3);
    const indices = this.indices.slice(0, this.used);
    const normals = new Float32Array(positions.length);
    for (let i = 0; i < indices.length; i += 3) {
      const a = indices[i]! * 3;
      const b = indices[i + 1]! * 3;
      const c = indices[i + 2]! * 3;
      const ux = positions[b]! - positions[a]!;
      const uy = positions[b + 1]! - positions[a + 1]!;
      const uz = positions[b + 2]! - positions[a + 2]!;
      const vx = positions[c]! - positions[a]!;
      const vy = positions[c + 1]! - positions[a + 1]!;
      const vz = positions[c + 2]! - positions[a + 2]!;
      // The cross product's length is twice the triangle's area: larger faces weigh more.
      const nx = uy * vz - uz * vy;
      const ny = uz * vx - ux * vz;
      const nz = ux * vy - uy * vx;
      normals[a] = normals[a]! + nx;
      normals[a + 1] = normals[a + 1]! + ny;
      normals[a + 2] = normals[a + 2]! + nz;
      normals[b] = normals[b]! + nx;
      normals[b + 1] = normals[b + 1]! + ny;
      normals[b + 2] = normals[b + 2]! + nz;
      normals[c] = normals[c]! + nx;
      normals[c + 1] = normals[c + 1]! + ny;
      normals[c + 2] = normals[c + 2]! + nz;
    }
    for (let i = 0; i < normals.length; i += 3) {
      const length = Math.hypot(normals[i]!, normals[i + 1]!, normals[i + 2]!);
      if (length > 0) {
        normals[i] = normals[i]! / length;
        normals[i + 1] = normals[i + 1]! / length;
        normals[i + 2] = normals[i + 2]! / length;
      } else {
        normals[i + 2] = 1;
      }
    }
    const empty = this.vertices === 0;
    return {
      positions,
      normals,
      indices,
      triangles: this.triangles,
      dropped: this.dropped,
      min: empty ? [0, 0, 0] : [...this.min],
      max: empty ? [0, 0, 0] : [...this.max],
    };
  }
}

/** All at once. Use in a worker, or for small files. */
export function parseBinaryStl(buffer: ArrayBuffer): ParsedMesh {
  const builder = new StlBuilder(buffer);
  builder.step(builder.triangles);
  return builder.finish();
}

/** In slices, giving the caller's `pause` a turn between them (main-thread fallback when no worker is available). */
export async function parseBinaryStlChunked(
  buffer: ArrayBuffer,
  pause: () => Promise<void>,
  onProgress?: (fraction: number) => void,
  slice = 40_000,
): Promise<ParsedMesh> {
  const builder = new StlBuilder(buffer);
  while (!builder.step(slice)) {
    onProgress?.(builder.progress / Math.max(1, builder.triangles));
    await pause();
  }
  return builder.finish();
}

/** Bytes a parsed mesh keeps alive; used for the cache budget. */
export function meshBytes(mesh: ParsedMesh): number {
  return mesh.positions.byteLength + mesh.normals.byteLength + mesh.indices.byteLength;
}
