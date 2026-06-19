import type { TransportClient, ForwardOutcome } from "./types.ts";
import type { MatrixF32 } from "../matrix.ts";

const MAGIC = 0x4b49524b;
const VERSION = 1;
const HEADER_LEN = 16;
const OP_FORWARD = 0x01;
const OP_ERROR = 0xff;

declare const Bun: {
  connect(opts: {
    hostname: string;
    port: number;
    socket: {
      data(socket: BunSocket, chunk: Uint8Array): void;
      open?(socket: BunSocket): void;
      close?(socket: BunSocket): void;
      error?(socket: BunSocket, err: Error): void;
    };
  }): Promise<BunSocket>;
};

interface BunSocket {
  write(data: Uint8Array | string): number;
  end(): void;
}

interface Pending {
  resolve: (out: ForwardOutcome) => void;
  reject: (err: Error) => void;
  bytesOut: number;
}

export class TcpClient implements TransportClient {
  private socket: BunSocket | null = null;
  private pending = new Map<number, Pending>();
  private buf = new Uint8Array(0);
  private nextReqId = 1;

  constructor(
    private host: string,
    private port: number,
  ) {}

  async open(): Promise<void> {
    if (typeof Bun === "undefined" || !Bun.connect) {
      throw new Error("TCP transport requires Bun runtime (Bun.connect)");
    }
    const self = this;
    this.socket = await Bun.connect({
      hostname: this.host,
      port: this.port,
      socket: {
        data(_s, chunk) {
          self.onData(chunk);
        },
        close() {
          for (const p of self.pending.values()) {
            p.reject(new Error("tcp socket closed"));
          }
          self.pending.clear();
        },
        error(_s, err) {
          for (const p of self.pending.values()) {
            p.reject(err);
          }
          self.pending.clear();
        },
      },
    });
  }

  private onData(chunk: Uint8Array): void {
    // Append to buffer and try to consume frames.
    const merged = new Uint8Array(this.buf.length + chunk.length);
    merged.set(this.buf, 0);
    merged.set(chunk, this.buf.length);
    this.buf = merged;

    while (this.buf.length >= HEADER_LEN) {
      const view = new DataView(this.buf.buffer, this.buf.byteOffset, HEADER_LEN);
      const magic = view.getUint32(0, true);
      if (magic !== MAGIC) {
        // Reset; the server is misbehaving.
        for (const p of this.pending.values()) p.reject(new Error("bad magic"));
        this.pending.clear();
        this.buf = new Uint8Array(0);
        return;
      }
      const opcode = view.getUint8(5);
      const reqId = view.getUint32(8, true);
      const payloadLen = view.getUint32(12, true);
      if (this.buf.length < HEADER_LEN + payloadLen) {
        return; // wait for more data
      }
      const payload = this.buf.subarray(HEADER_LEN, HEADER_LEN + payloadLen);
      const totalLen = HEADER_LEN + payloadLen;
      const handler = this.pending.get(reqId);
      this.pending.delete(reqId);
      if (handler) {
        if (opcode === OP_ERROR) {
          const codeView = new DataView(payload.buffer, payload.byteOffset, payload.byteLength);
          const code = codeView.getUint16(0, true);
          const msgLen = codeView.getUint32(4, true);
          const msgBytes = payload.subarray(8, 8 + msgLen);
          const msg = new TextDecoder().decode(msgBytes);
          handler.reject(new Error(`tcp error code=0x${code.toString(16)} msg=${msg}`));
        } else {
          handler.resolve({ bytesIn: totalLen, bytesOut: handler.bytesOut });
        }
      }
      this.buf = this.buf.subarray(totalLen);
    }
  }

  async forward(matrix: MatrixF32, timestampUs: bigint): Promise<ForwardOutcome> {
    if (!this.socket) throw new Error("tcp socket not opened");
    const n = matrix.dim;
    const payloadLen = 4 + 4 * n * n + 4 * n * n + 8;
    const frameLen = HEADER_LEN + payloadLen;
    const frame = new Uint8Array(frameLen);
    const view = new DataView(frame.buffer);
    const reqId = this.nextReqId++;
    if (this.nextReqId > 0xffff_fffe) this.nextReqId = 1;
    // header
    view.setUint32(0, MAGIC, true);
    frame[4] = VERSION;
    frame[5] = OP_FORWARD;
    view.setUint16(6, 0, true); // flags
    view.setUint32(8, reqId, true);
    view.setUint32(12, payloadLen, true);
    // payload: n
    view.setUint32(HEADER_LEN, n, true);
    // matrix_re
    const reBytes = new Uint8Array(matrix.re.buffer, matrix.re.byteOffset, matrix.re.byteLength);
    frame.set(reBytes, HEADER_LEN + 4);
    // matrix_im
    const imBytes = new Uint8Array(matrix.im.buffer, matrix.im.byteOffset, matrix.im.byteLength);
    frame.set(imBytes, HEADER_LEN + 4 + reBytes.length);
    // timestamp_us
    view.setBigInt64(HEADER_LEN + 4 + reBytes.length + imBytes.length, timestampUs, true);

    return await new Promise<ForwardOutcome>((resolve, reject) => {
      this.pending.set(reqId, { resolve, reject, bytesOut: frame.length });
      this.socket!.write(frame);
    });
  }

  async close(): Promise<void> {
    if (this.socket) {
      this.socket.end();
      this.socket = null;
    }
  }
}
