import type { MatrixF32 } from "../matrix.ts";

export interface ForwardOutcome {
  bytesIn: number;
  bytesOut: number;
}

export interface TransportClient {
  /** Open required resources (sockets, channels). */
  open(): Promise<void>;
  /** Issue a single forward call. Throws on transport error. */
  forward(matrix: MatrixF32, timestampUs: bigint): Promise<ForwardOutcome>;
  /** Release all resources. */
  close(): Promise<void>;
}
