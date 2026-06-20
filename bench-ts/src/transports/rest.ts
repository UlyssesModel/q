import type { TransportClient, ForwardOutcome } from "./types.ts";
import type { MatrixF32 } from "../matrix.ts";

function b64encode(buf: ArrayBufferView): string {
  // Bun has a fast btoa(); for binary safety go via Uint8Array → string of latin-1.
  const u8 = new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  let s = "";
  for (let i = 0; i < u8.length; i++) {
    s += String.fromCharCode(u8[i]);
  }
  return btoa(s);
}

function buildV2Matrix(matrix: MatrixF32): number[][][] {
  const n = matrix.dim;
  const out: number[][][] = new Array(n);
  for (let r = 0; r < n; r++) {
    const row: number[][] = new Array(n);
    const base = r * n;
    for (let c = 0; c < n; c++) {
      row[c] = [matrix.re[base + c], matrix.im[base + c]];
    }
    out[r] = row;
  }
  return out;
}

export class RestClient implements TransportClient {
  constructor(
    private host: string,
    private port: number,
    private apiVersion: "v1" | "v2" = "v1",
  ) {}

  async open(): Promise<void> {
    // fetch handles keepalive internally on Bun; nothing explicit.
  }

  async forward(matrix: MatrixF32, timestampUs: bigint): Promise<ForwardOutcome> {
    const body = this.apiVersion === "v2"
      ? JSON.stringify({
          matrix: buildV2Matrix(matrix),
          timestamp_us: Number(timestampUs),
        })
      : JSON.stringify({
          matrix_re: b64encode(matrix.re),
          matrix_im: b64encode(matrix.im),
          matrix_dim: matrix.dim,
          timestamp_us: Number(timestampUs),
        });
    const url = `http://${this.host}:${this.port}/${this.apiVersion}/forward`;
    const res = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json", connection: "keep-alive" },
      body,
    });
    if (!res.ok) {
      throw new Error(`rest ${this.apiVersion} forward HTTP ${res.status}`);
    }
    const text = await res.text();
    return { bytesIn: text.length, bytesOut: body.length };
  }

  async close(): Promise<void> {
    // No persistent socket to release explicitly.
  }
}
