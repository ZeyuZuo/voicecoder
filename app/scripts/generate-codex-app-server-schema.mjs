import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const codexBin = process.env.VOICECODER_CODEX_BIN?.trim() || "codex";
const outputRoot = resolve("src-tauri/generated/codex-app-server");
const jsonOutput = resolve(outputRoot, "json");
const typescriptOutput = resolve(outputRoot, "typescript");

const version = runCodex(["--version"]).trim();

rmSync(outputRoot, { force: true, recursive: true });
mkdirSync(jsonOutput, { recursive: true });
mkdirSync(typescriptOutput, { recursive: true });

runCodex(["app-server", "generate-json-schema", "--out", jsonOutput]);
runCodex(["app-server", "generate-ts", "--out", typescriptOutput]);

writeFileSync(
  resolve(outputRoot, "manifest.json"),
  `${JSON.stringify({ generatedAt: new Date().toISOString(), version, transport: "stdio" }, null, 2)}\n`
);

console.log(`Generated Codex app-server schemas for ${version} at ${outputRoot}`);

function runCodex(args) {
  const result = spawnSync(codexBin, args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"]
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    throw new Error(`Codex exited with status ${result.status}: ${codexBin} ${args.join(" ")}`);
  }

  return result.stdout;
}
