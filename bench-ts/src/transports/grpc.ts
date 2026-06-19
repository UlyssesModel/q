import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import type { TransportClient, ForwardOutcome } from "./types.ts";
import type { MatrixF32 } from "../matrix.ts";

// Lazy import — only resolved at runtime so `bunx tsc --noEmit` does not require
// the grpc-js types to be installed.
type GrpcModule = typeof import("@grpc/grpc-js");
type ProtoLoaderModule = typeof import("@grpc/proto-loader");

let grpcRef: GrpcModule | null = null;
let loaderRef: ProtoLoaderModule | null = null;

async function ensureGrpc(): Promise<{ grpc: GrpcModule; loader: ProtoLoaderModule }> {
  if (!grpcRef) {
    grpcRef = (await import("@grpc/grpc-js")) as unknown as GrpcModule;
  }
  if (!loaderRef) {
    loaderRef = (await import("@grpc/proto-loader")) as unknown as ProtoLoaderModule;
  }
  return { grpc: grpcRef, loader: loaderRef };
}

function protoPath(): string {
  const here = dirname(fileURLToPath(import.meta.url));
  return resolve(here, "..", "..", "..", "proto", "kirk.proto");
}

interface KirkServiceClient {
  Forward(
    req: unknown,
    cb: (err: Error | null, res: unknown) => void,
  ): void;
  close(): void;
}

export class GrpcClient implements TransportClient {
  private client: KirkServiceClient | null = null;

  constructor(
    private host: string,
    private port: number,
  ) {}

  async open(): Promise<void> {
    const { grpc, loader } = await ensureGrpc();
    const packageDefinition = loader.loadSync(protoPath(), {
      keepCase: true,
      longs: String,
      enums: String,
      defaults: true,
      oneofs: true,
    });
    const proto = grpc.loadPackageDefinition(packageDefinition) as unknown as {
      kirk: { v1: { KirkService: new (addr: string, creds: unknown, opts: unknown) => KirkServiceClient } };
    };
    const ctor = proto.kirk.v1.KirkService;
    this.client = new ctor(
      `${this.host}:${this.port}`,
      grpc.credentials.createInsecure(),
      {
        "grpc.keepalive_time_ms": 15000,
        "grpc.max_concurrent_streams": 1024,
      },
    );
  }

  async forward(matrix: MatrixF32, timestampUs: bigint): Promise<ForwardOutcome> {
    if (!this.client) throw new Error("grpc client not opened");
    const reBytes = Buffer.from(matrix.re.buffer, matrix.re.byteOffset, matrix.re.byteLength);
    const imBytes = Buffer.from(matrix.im.buffer, matrix.im.byteOffset, matrix.im.byteLength);
    const req = {
      matrix: { dim: matrix.dim, data_re: reBytes, data_im: imBytes },
      timestamp_us: Number(timestampUs),
    };
    return await new Promise<ForwardOutcome>((resolve, reject) => {
      this.client!.Forward(req, (err, _res) => {
        if (err) {
          reject(err);
          return;
        }
        resolve({
          bytesIn: reBytes.length + imBytes.length + 24, // rough upper bound on response payload prefix
          bytesOut: reBytes.length + imBytes.length + 16,
        });
      });
    });
  }

  async close(): Promise<void> {
    if (this.client) {
      this.client.close();
      this.client = null;
    }
  }
}
