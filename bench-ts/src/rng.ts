// Seeded xoshiro256** RNG. Pure TS; matches rand_xoshiro on the Rust side bit
// for bit when seeded from a u64.

const MASK64 = (1n << 64n) - 1n;

function splitmix64(seed: bigint): { state: bigint; next: bigint } {
  let z = (seed + 0x9e3779b97f4a7c15n) & MASK64;
  z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK64;
  z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK64;
  z = z ^ (z >> 31n);
  return { state: (seed + 0x9e3779b97f4a7c15n) & MASK64, next: z & MASK64 };
}

export class Xoshiro256ss {
  private s: [bigint, bigint, bigint, bigint];

  constructor(seed: bigint) {
    let cur = seed;
    const state: bigint[] = [];
    for (let i = 0; i < 4; i++) {
      const r = splitmix64(cur);
      cur = r.state;
      state.push(r.next);
    }
    this.s = [state[0], state[1], state[2], state[3]];
  }

  private rotl(x: bigint, k: bigint): bigint {
    return ((x << k) | (x >> (64n - k))) & MASK64;
  }

  nextU64(): bigint {
    const result = (this.rotl((this.s[1] * 5n) & MASK64, 7n) * 9n) & MASK64;
    const t = (this.s[1] << 17n) & MASK64;
    this.s[2] ^= this.s[0];
    this.s[3] ^= this.s[1];
    this.s[1] ^= this.s[2];
    this.s[0] ^= this.s[3];
    this.s[2] ^= t;
    this.s[3] = this.rotl(this.s[3], 45n);
    return result;
  }

  /** Returns a float in [0, 1). */
  nextFloat(): number {
    // Take 53 high bits.
    const x = this.nextU64() >> 11n;
    return Number(x) / 2 ** 53;
  }

  /** Returns a float in [-1, 1). */
  nextSigned(): number {
    return this.nextFloat() * 2 - 1;
  }
}
