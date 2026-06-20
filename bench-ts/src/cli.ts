#!/usr/bin/env bun
import { parseArgs } from "node:util";
import { run } from "./runner.ts";
import { compare } from "./compare.ts";
import type { RunOptions } from "./results.ts";

function usage(): never {
  console.log(`usage:
  bench run    --transport <grpc|rest|tcp> [--api-version v1|v2]
               [--host localhost] [--port N]
               [--users 10] [--duration 30s | --requests N]
               [--matrix-size 32] [--temperature 1.0]
               [--seed 42] [--warmup 2s] [--output path]
               [--op forward]
  bench compare <result.json> [...]

  --api-version applies only to --transport rest. v1 (default) uses
  the base64 envelope; v2 uses nested JSON arrays. Ignored for grpc
  and tcp transports.
`);
  process.exit(2);
}

async function main() {
  const argv = process.argv.slice(2);
  if (argv.length === 0) usage();
  const sub = argv[0];
  if (sub === "compare") {
    await compare(argv.slice(1));
    return;
  }
  if (sub !== "run") usage();

  const { values } = parseArgs({
    args: argv.slice(1),
    options: {
      transport: { type: "string" },
      "api-version": { type: "string" },
      host: { type: "string" },
      port: { type: "string" },
      users: { type: "string" },
      duration: { type: "string" },
      requests: { type: "string" },
      "matrix-size": { type: "string" },
      temperature: { type: "string" },
      seed: { type: "string" },
      warmup: { type: "string" },
      output: { type: "string" },
      op: { type: "string" },
    },
    allowPositionals: true,
  });

  if (!values.transport || !["grpc", "rest", "tcp"].includes(values.transport)) {
    console.error("--transport must be one of grpc|rest|tcp");
    usage();
  }

  const apiVersion = values["api-version"] ?? "v1";
  if (!["v1", "v2"].includes(apiVersion)) {
    console.error("--api-version must be v1 or v2");
    usage();
  }
  if (values.transport !== "rest" && values["api-version"] && apiVersion !== "v1") {
    console.error(`--api-version=${apiVersion} only applies to --transport rest; ignoring`);
  }

  const opts: Partial<RunOptions> = {
    transport: values.transport as "grpc" | "rest" | "tcp",
    apiVersion: apiVersion as "v1" | "v2",
    host: values.host,
    port: values.port ? Number(values.port) : undefined,
    users: values.users ? Number(values.users) : undefined,
    duration: values.duration,
    requests: values.requests ? Number(values.requests) : undefined,
    matrixSize: values["matrix-size"] ? Number(values["matrix-size"]) : undefined,
    temperature: values.temperature ? Number(values.temperature) : undefined,
    seed: values.seed ? BigInt(values.seed) : undefined,
    warmup: values.warmup,
    output: values.output,
    op: values.op,
  };
  await run(opts);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
