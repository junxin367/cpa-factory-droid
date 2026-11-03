import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pluginId = "factory-droid";
const packageName = "cpa-factory-droid";
const repository = "junxin367/cpa-factory-droid";
const platforms = [
  { os: "windows", arch: "amd64" },
  { os: "linux", arch: "amd64" },
  { os: "darwin", arch: "amd64" },
  { os: "darwin", arch: "arm64" },
];
const semanticVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$/;

function execute(command, args) {
  return execFileSync(command, args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function packageVersion() {
  const metadata = JSON.parse(execute("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"]));
  const packages = metadata.packages.filter((entry) => entry.name === packageName);
  if (packages.length !== 1) throw new Error(`Cargo workspace 必须包含唯一的 ${packageName} 包。`);
  return packages[0].version;
}

function parseTag(tag) {
  if (typeof tag !== "string" || !tag.startsWith("v") || !semanticVersion.test(tag.slice(1))) {
    throw new Error(`版本标签必须为 v0.1.0 形式，实际：${tag ?? ""}`);
  }
  return tag.slice(1);
}

function assetsFor(version) {
  if (!semanticVersion.test(version)) throw new Error(`版本号无效：${version}`);
  return platforms.map(({ os, arch }) => `${pluginId}_${version}_${os}_${arch}.zip`);
}

function sourceSnapshot() {
  const paths = execute("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
    .split("\0").filter(Boolean);
  const files = [...new Set(paths)].sort().map((path) => ({
    path,
    sha256: createHash("sha256").update(readFileSync(join(root, path))).digest("hex"),
  }));
  const sha256 = createHash("sha256")
    .update(files.map(({ path, sha256: hash }) => `${path}\0${hash}\n`).join(""))
    .digest("hex");
  return { sha256, files };
}

function localPackage(archive, target, expectedSource, libraryHash) {
  const version = packageVersion();
  const source = sourceSnapshot();
  if (source.sha256 !== expectedSource) throw new Error("构建期间源码发生变化，需从稳定源码重新打包。");
  if (!/^[a-f0-9]{64}$/.test(libraryHash ?? "")) throw new Error("动态库 SHA-256 无效。");
  const targets = {
    "x86_64-pc-windows-msvc": { os: "windows", arch: "amd64", extension: "dll" },
    "x86_64-unknown-linux-gnu": { os: "linux", arch: "amd64", extension: "so" },
    "x86_64-apple-darwin": { os: "darwin", arch: "amd64", extension: "dylib" },
    "aarch64-apple-darwin": { os: "darwin", arch: "arm64", extension: "dylib" },
  };
  const platform = targets[target];
  if (!platform) throw new Error(`不支持的构建目标：${target}`);
  const filename = archive.split(/[\\/]/).at(-1);
  const allowed = [`${pluginId}_${version}_${platform.os}_${platform.arch}.zip`,
    `${pluginId}_${version}_${platform.os}_${platform.arch}_debug.zip`];
  if (!allowed.includes(filename)) throw new Error("安装包文件名与版本或构建平台不一致。");
  const bytes = readFileSync(archive);
  const archiveHash = createHash("sha256").update(bytes).digest("hex");
  const line = `${archiveHash}  ${filename}\n`;
  let commit = null;
  let branch = null;
  try {
    commit = execute("git", ["rev-parse", "--verify", "HEAD"]);
  } catch {
    // 首次提交前可构建本地审阅包，不生成虚构提交号。
  }
  try {
    branch = execute("git", ["symbolic-ref", "--quiet", "--short", "HEAD"]);
  } catch {
    // 标签构建处于 detached HEAD。
  }
  const payload = {
    schema_version: 1,
    plugin_id: pluginId,
    package_name: packageName,
    version,
    target,
    built_at: new Date().toISOString(),
    published: false,
    git: {
      commit,
      branch,
      dirty: execute("git", ["status", "--porcelain"]).length > 0,
    },
    rustc: execute("rustc", ["--version"]),
    archive: { name: filename, size: bytes.length, sha256: archiveHash },
    library: { name: `${pluginId}.${platform.extension}`, sha256: libraryHash },
    source,
  };
  writeFileSync(`${archive}.sha256`, line);
  writeFileSync(`${archive}.build.json`, `${JSON.stringify(payload, null, 2)}\n`);
  const outputDirectory = dirname(archive);
  const present = readdirSync(outputDirectory);
  const localChecksums = assetsFor(version).filter((name) => present.includes(name)).sort().map((name) =>
    `${createHash("sha256").update(readFileSync(join(outputDirectory, name))).digest("hex")}  ${name}`);
  if (localChecksums.length) writeFileSync(join(outputDirectory, "checksums.txt"), `${localChecksums.join("\n")}\n`);
  process.stdout.write(`本地构建清单：${archive}.build.json\nZIP SHA-256：${archiveHash}\n源码 SHA-256：${source.sha256}\n`);
}

function metadata() {
  const version = packageVersion();
  const sha = execute("git", ["rev-parse", "--verify", "HEAD"]);
  const tag = process.env.RELEASE_TAG ?? "";
  if (tag) {
    if (parseTag(tag) !== version) throw new Error(`标签 ${tag} 与 Cargo 版本 ${version} 不一致。`);
    const taggedSha = execute("git", ["rev-parse", "--verify", `refs/tags/${tag}^{commit}`]);
    if (taggedSha !== sha) throw new Error("当前提交与发布标签指向的提交不一致。");
  }
  const output = `tag=${tag}\nversion=${version}\nsha=${sha}\n`;
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, output);
  process.stdout.write(output);
}

function registry() {
  const document = JSON.parse(readFileSync(join(root, "registry.json"), "utf8"));
  if (document.schema_version !== 1 || !Array.isArray(document.plugins)) {
    throw new Error("registry.json 必须包含 schema_version: 1 和 plugins 数组。");
  }
  const entries = document.plugins.filter((entry) => entry?.id === pluginId);
  if (entries.length !== 1) throw new Error(`registry.json 必须包含唯一的 ${pluginId}。`);
  const plugin = entries[0];
  if (plugin.version !== packageVersion()) throw new Error("registry.json 与 Cargo.toml 的版本不一致。");
  if (plugin.repository?.replace(/\/$/, "") !== `https://github.com/${repository}`) {
    throw new Error("registry.json 的 repository 与本项目不一致。");
  }
  for (const field of ["name", "description", "author", "license"]) {
    if (typeof plugin[field] !== "string" || !plugin[field].trim()) {
      throw new Error(`registry.json 中 ${field} 不能为空。`);
    }
  }
  process.stdout.write(`注册表检查通过：${pluginId}@${plugin.version}\n`);
}

function checksums(directory, version) {
  if (!directory) throw new Error("必须提供发布产物目录。");
  const expected = assetsFor(version).sort();
  const actual = readdirSync(directory).filter((name) => !["checksums.txt", "latest.json"].includes(name)).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`发布产物列表不完整或包含额外文件。\n预期：${expected.join(", ")}\n实际：${actual.join(", ")}`);
  }
  const lines = expected.map((name) => {
    const bytes = readFileSync(join(directory, name));
    if (!bytes.length) throw new Error(`发布产物为空：${name}`);
    return `${createHash("sha256").update(bytes).digest("hex")}  ${name}`;
  });
  writeFileSync(join(directory, "checksums.txt"), `${lines.join("\n")}\n`);
}

function manifest(input, output) {
  const release = JSON.parse(readFileSync(input, "utf8"));
  const version = parseTag(release.tagName);
  const expected = [...assetsFor(version), "checksums.txt"];
  const files = (release.assets ?? []).filter((asset) => expected.includes(asset.name));
  for (const name of expected) {
    const matches = files.filter((asset) => asset.name === name);
    if (matches.length !== 1 || !matches[0].url) throw new Error(`Release 缺少唯一的可下载产物：${name}`);
  }
  if (!release.url) throw new Error("Release 缺少页面地址。");
  const payload = {
    version: `v${version}`,
    // Draft html_url may use an untagged URL; the published address is fixed.
    url: `https://github.com/${repository}/releases/tag/${release.tagName}`,
    body: release.body ?? "",
    assets: files.map(({ name, url }) => ({ name, url })).sort((a, b) => a.name.localeCompare(b.name)),
  };
  writeFileSync(output, `${JSON.stringify(payload, null, 2)}\n`);
}

function notes(tag) {
  const version = parseTag(tag);
  const curated = join(root, "docs", "releases", `${tag}.md`);
  try {
    const text = readFileSync(curated, "utf8");
    if (!text.includes(tag)) throw new Error("发布说明与标签不一致。");
    process.stdout.write(text);
    return;
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  let previous = "";
  try {
    previous = execute("git", ["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", `${tag}^`]);
  } catch {
    // 首次发布没有上一个标签。
  }
  const subjects = execute("git", ["log", "--format=%s", previous ? `${previous}..${tag}` : tag])
    .split(/\r?\n/).filter(Boolean);
  const base = `https://github.com/${repository}`;
  const lines = [
    `## ${tag}`, "",
    `CLIProxyAPI 原生 Factory Droid provider 插件 ${version}。`, "",
    "### 📝 变更", "",
    ...subjects.map((subject) => `- ${subject}`), "",
    "### 📦 安装包", "",
    ...assetsFor(version).map((name) => `- \`${name}\``), "",
    "按 README 添加本仓库的 registry.json 插件源并安装，或手动安装对应系统的动态库。",
    "`checksums.txt` 提供 ZIP 文件的 SHA-256；每个 ZIP 内另含动态库的 `SHA256SUMS.txt`。", "",
    `[安装与配置说明](${base}/blob/${tag}/README.md)`,
    previous ? `[完整变更](${base}/compare/${previous}...${tag})` : `[版本提交](${base}/commits/${tag})`,
    "",
  ];
  process.stdout.write(lines.join("\n"));
}

const [command, ...args] = process.argv.slice(2);
switch (command) {
  case "snapshot": process.stdout.write(`${sourceSnapshot().sha256}\n`); break;
  case "local": localPackage(...args); break;
  case "metadata": metadata(); break;
  case "registry": registry(); break;
  case "checksums": checksums(...args); break;
  case "manifest": manifest(...args); break;
  case "notes": notes(...args); break;
  default: throw new Error("用法：node scripts/release.mjs metadata|registry|snapshot|local <ZIP> <target> <source-sha256> <library-sha256>|checksums <目录> <版本>|manifest <release.json> <输出>|notes <标签>");
}
