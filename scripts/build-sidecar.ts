import { spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";

function getHostTargetTriple(): string {
  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout) {
    throw new Error(`Failed to execute rustc -vV: ${result.stderr}`);
  }
  const lines = result.stdout.split("\n");
  for (const line of lines) {
    if (line.startsWith("host: ")) {
      return line.substring("host: ".length).trim();
    }
  }
  throw new Error("Could not detect host triple from rustc -vV output");
}

function parseArgs(): { targetTriple: string; isRelease: boolean; customTarget: boolean } {
  const args = process.argv.slice(2);
  let targetTriple = "";
  let isRelease = true;
  let customTarget = false;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--target" && i + 1 < args.length) {
      targetTriple = args[i + 1];
      customTarget = true;
      i++;
    } else if (args[i].startsWith("--target=")) {
      targetTriple = args[i].substring("--target=".length);
      customTarget = true;
    } else if (args[i] === "--debug") {
      isRelease = false;
    } else if (args[i] === "--release") {
      isRelease = true;
    }
  }

  if (!targetTriple) {
    targetTriple = getHostTargetTriple();
  }

  return { targetTriple, isRelease, customTarget };
}

function main() {
  const { targetTriple, isRelease, customTarget } = parseArgs();
  console.log(`[build-sidecar] Preparing cdus-agent sidecar for target: ${targetTriple} (release: ${isRelease})`);

  const isWindows = targetTriple.includes("windows");
  const ext = isWindows ? ".exe" : "";
  const binaryName = `cdus-agent${ext}`;

  const cargoArgs = ["build", "--package", "cdus-agent"];
  if (isRelease) {
    cargoArgs.push("--release");
  }
  if (customTarget) {
    cargoArgs.push("--target", targetTriple);
  }

  console.log(`[build-sidecar] Running: cargo ${cargoArgs.join(" ")}`);
  const buildResult = spawnSync("cargo", cargoArgs, {
    stdio: "inherit",
    shell: process.platform === "win32",
  });

  if (buildResult.status !== 0) {
    console.error(`[build-sidecar] Cargo build failed with status ${buildResult.status}`);
    process.exit(buildResult.status ?? 1);
  }

  const profileDir = isRelease ? "release" : "debug";
  const sourceBinary = customTarget
    ? path.join(process.cwd(), "target", targetTriple, profileDir, binaryName)
    : path.join(process.cwd(), "target", profileDir, binaryName);

  if (!fs.existsSync(sourceBinary)) {
    console.error(`[build-sidecar] Expected binary not found at: ${sourceBinary}`);
    process.exit(1);
  }

  const outDir = path.join(process.cwd(), "src-tauri", "binaries");
  if (!fs.existsSync(outDir)) {
    fs.mkdirSync(outDir, { recursive: true });
  }

  const destinationBinary = path.join(outDir, `cdus-agent-${targetTriple}${ext}`);
  console.log(`[build-sidecar] Copying ${sourceBinary} -> ${destinationBinary}`);
  fs.copyFileSync(sourceBinary, destinationBinary);

  if (!isWindows) {
    try {
      fs.chmodSync(destinationBinary, 0o755);
    } catch (err) {
      console.warn(`[build-sidecar] Could not chmod ${destinationBinary}: ${err}`);
    }
  }

  console.log(`[build-sidecar] Successfully packaged cdus-agent sidecar for ${targetTriple}`);
}

main();
