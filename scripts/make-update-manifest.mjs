#!/usr/bin/env node
/**
 * 生成自动更新清单 `latest.json`（Tauri NSIS 安装包）。
 *
 * 用法（在 prism3 目录）：
 *   node scripts/make-update-manifest.mjs --base https://cdn.example.com/prism3
 *   node scripts/make-update-manifest.mjs --base https://cdn.example.com/prism3 --version 0.2.0
 *   node scripts/make-update-manifest.mjs --version 0.2.0 --redirect https://cdn.example.com/prism3/latest.json
 *
 * 选项：
 *   --base <url>       安装包下载基址（与 --exe 二选一；生成 binaries[].url = <base>/<文件名>）
 *   --version <x.y.z>  清单版本号（默认读 src-tauri/tauri.conf.json）
 *   --exe <path>       指定安装包（默认扫描 src-tauri/target/release/bundle/nsis/*-setup.exe）
 *   --out <path>       输出路径（默认 dist/latest.json；Gitee 跳板用 --out gitee-latest.json）
 *   --redirect <url>   生成「跳板指针」：{version, redirect, binaries: []}
 *   --notes <text>     发布说明（可选）
 *
 * 输出格式与 prism2 的 `latest.json` 兼容（见 docs/自动更新.md）：
 *   { version, notes, redirect, mirrors: [], binaries: [{ platform, arch, url, sha256 }] }
 *
 * 这是**发版工具**，不参与运行时；客户端只认清单，不依赖本脚本。
 */
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);

function argValue(name) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
}

function hasFlag(name) {
  return args.includes(name);
}

if (hasFlag("--help") || hasFlag("-h")) {
  console.log(readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n").slice(1, 28).join("\n"));
  process.exit(0);
}

const version =
  argValue("--version") ??
  JSON.parse(readFileSync(join(repoRoot, "src-tauri", "tauri.conf.json"), "utf8")).version;
const redirect = argValue("--redirect");
const notes = argValue("--notes") ?? "";
const base = argValue("--base");
const out = resolve(repoRoot, argValue("--out") ?? "dist/latest.json");

if (!version || !/^\d+\.\d+\.\d+([.-][0-9A-Za-z.-]+)?$/.test(String(version))) {
  console.error(`版本号不合法：${version}（应形如 0.2.0）`);
  process.exit(1);
}

if (redirect) {
  const url = normalizeBase(redirect);
  const manifest = {
    version: String(version),
    notes,
    redirect: url,
    mirrors: [],
    binaries: [],
  };
  writeManifest(out, manifest);
  process.exit(0);
}

if (!base) {
  console.error("缺少 --base（安装包下载基址）或 --redirect（跳板指针）");
  process.exit(1);
}
const baseUrl = normalizeBase(base);

const exe = resolveInstaller(argValue("--exe"));
const name = basename(exe);
const manifest = {
  version: String(version),
  notes,
  redirect: null,
  mirrors: [],
  binaries: [
    {
      platform: "win",
      arch: detectArch(name),
      url: `${baseUrl}/${encodeURIComponent(name)}`,
      sha256: sha256File(exe),
    },
  ],
};
writeManifest(out, manifest);

function normalizeBase(value) {
  const trimmed = String(value).trim().replace(/\/+$/, "");
  if (!/^https?:\/\//i.test(trimmed)) {
    console.error(`地址必须以 http(s):// 开头：${value}`);
    process.exit(1);
  }
  return trimmed;
}

function resolveInstaller(explicit) {
  if (explicit) {
    const path = resolve(repoRoot, explicit);
    if (!existsSync(path) || !statSync(path).isFile()) {
      console.error(`安装包不存在：${path}`);
      process.exit(1);
    }
    return path;
  }

  const dir = join(repoRoot, "src-tauri", "target", "release", "bundle", "nsis");
  if (!existsSync(dir)) {
    console.error(
      `未找到 NSIS 产物目录：${dir}\n请先执行 pnpm tauri build，或用 --exe 指定安装包。`,
    );
    process.exit(1);
  }
  const candidates = readdirSync(dir)
    .filter((entry) => entry.toLowerCase().endsWith("-setup.exe"))
    .map((entry) => join(dir, entry))
    .sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs);
  if (candidates.length === 0) {
    console.error(`目录里没有 *-setup.exe：${dir}`);
    process.exit(1);
  }
  return candidates[0];
}

function basename(path) {
  return path.split(/[\\/]/).pop();
}

function detectArch(name) {
  const lower = name.toLowerCase();
  if (lower.includes("arm64")) return "arm64";
  if (lower.includes("universal")) return "universal";
  if (lower.includes("x64") || lower.includes("x86_64")) return "x64";
  return "x64";
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function writeManifest(path, manifest) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  console.log(`已生成 ${path}`);
  console.log(JSON.stringify(manifest, null, 2));
}
