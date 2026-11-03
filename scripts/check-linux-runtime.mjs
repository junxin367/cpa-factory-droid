import { execFileSync } from "node:child_process";

const [library, maximum = "2.35"] = process.argv.slice(2);
if (!library || !/^\d+\.\d+(?:\.\d+)?$/.test(maximum)) {
  throw new Error("用法：node scripts/check-linux-runtime.mjs <插件.so> [最高允许的GLIBC版本，默认2.35]");
}
const readelf = (...args) => execFileSync("readelf", [...args, library], { encoding: "utf8" });
const versionInfo = readelf("--version-info", "--wide");
const versions = [...new Set([...versionInfo.matchAll(/\bName: GLIBC_(\d+(?:\.\d+)+)\b/g)]
  .map((match) => match[1]))];
const compare = (left, right) => {
  const a = left.split(".").map(Number);
  const b = right.split(".").map(Number);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const difference = (a[index] ?? 0) - (b[index] ?? 0);
    if (difference) return difference;
  }
  return 0;
};
if (!versions.length) throw new Error("未找到 GLIBC 导入版本，无法验证 Linux 运行时要求。");
const highest = versions.sort(compare).at(-1);
if (compare(highest, maximum) > 0) {
  throw new Error(`插件需要 GLIBC ${highest}，超过允许的 ${maximum} 基线。`);
}
const libraries = [...readelf("--dynamic", "--wide").matchAll(/\(NEEDED\).*?\[([^\]]+)\]/g)]
  .map((match) => match[1]);
process.stdout.write(`GLIBC 导入最高版本：${highest}，允许基线：${maximum}\n动态依赖：${libraries.join(", ")}\n`);
