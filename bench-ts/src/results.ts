import { mkdir, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { Summary } from "./summary.ts";

export interface RunOptions {
  transport: "grpc" | "rest" | "tcp";
  apiVersion: "v1" | "v2";
  host: string;
  port: number;
  users: number;
  duration?: string;
  requests?: number;
  matrixSize: number;
  temperature: number;
  seed: bigint;
  warmup: string;
  output: string;
  op: string;
}

export interface ResultFile {
  meta: {
    transport: string;
    api_version: string;
    host: string;
    port: number;
    users: number;
    matrix_size: number;
    op: string;
    seed: string;
    warmup: string;
    duration?: string;
    requests?: number;
    started_at: string;
    finished_at: string;
  };
  summary: Summary;
}

export async function writeResult(path: string, file: ResultFile): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, JSON.stringify(file, null, 2), "utf-8");
}
