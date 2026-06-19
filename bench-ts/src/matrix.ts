import { Xoshiro256ss } from "./rng.ts";

export interface MatrixF32 {
  re: Float32Array;
  im: Float32Array;
  dim: number;
}

export function generateMatrix(rng: Xoshiro256ss, dim: number): MatrixF32 {
  const len = dim * dim;
  const re = new Float32Array(len);
  const im = new Float32Array(len);
  for (let i = 0; i < len; i++) {
    re[i] = rng.nextSigned();
    im[i] = rng.nextSigned();
  }
  return { re, im, dim };
}

/** Pre-generated circular buffer of matrices (warmup-phase population). */
export class MatrixPool {
  private mats: MatrixF32[];
  private idx = 0;

  constructor(rng: Xoshiro256ss, dim: number, count: number) {
    this.mats = [];
    for (let i = 0; i < count; i++) {
      this.mats.push(generateMatrix(rng, dim));
    }
  }

  next(): MatrixF32 {
    const m = this.mats[this.idx];
    this.idx = (this.idx + 1) % this.mats.length;
    return m;
  }
}
