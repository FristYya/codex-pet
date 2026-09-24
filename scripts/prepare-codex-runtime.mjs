import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { copyFile, cp, readFile, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const version = "0.156.1-win32-x64";
const packageName = `@openai/codex@${version}`;
const integrity = "sha512-MJyLxbBs2zzp5kbaR/99Zwe7SmbrwUkveTcT+ayYlO48V0nYh0eU+h2lalBwvC7VJ/ya/bXnUtISJfJKhGCD/g==";
const root = fileURLToPath(new URL("..", import.meta.url));
const cache = join(root, ".runtime-cache");
const destination = join(root, "src-tauri", "resources", "codex-runtime");
const license = join(root, "scripts", "licenses", "CODEX-RUNTIME-LICENSE.txt");
const archive = join(cache, "openai-codex-0.156.1-win32-x64.tgz");
const extracted = join(cache, "package");

if (!existsSync(license)) throw new Error("Codex runtime Apache-2.0 license file is missing");
rmSync(cache, { recursive: true, force: true });
mkdirSync(cache, { recursive: true });
const npm = process.platform === "win32"
  ? { executable: "cmd.exe", prefix: ["/d", "/s", "/c", "npm.cmd"] }
  : { executable: "npm", prefix: [] };
execFileSync(npm.executable, [...npm.prefix, "pack", packageName, "--pack-destination", cache], {
  cwd: root,
  stdio: "inherit",
});

if (!existsSync(archive)) throw new Error(`官方 runtime 下载文件不存在：${basename(archive)}`);
const digest = createHash("sha512").update(await readFile(archive)).digest("base64");
if (`sha512-${digest}` !== integrity) throw new Error("官方 Codex runtime 完整性校验失败");

execFileSync(process.platform === "win32" ? "tar.exe" : "tar", ["-xzf", archive, "-C", cache], { stdio: "inherit" });
const vendor = join(extracted, "vendor", "x86_64-pc-windows-msvc");
if (!existsSync(join(vendor, "bin", "codex.exe"))) throw new Error("官方 runtime 包缺少 codex.exe");

rmSync(destination, { recursive: true, force: true });
mkdirSync(destination, { recursive: true });
await cp(vendor, destination, { recursive: true });
await copyFile(license, join(destination, "CODEX-RUNTIME-LICENSE.txt"));
await writeFile(join(destination, ".gitkeep"), "");
const unexpected = readdirSync(destination).length === 0;
if (unexpected) throw new Error("官方 runtime 提取为空");
console.log(`已准备官方 bundled Codex runtime ${version}`);
