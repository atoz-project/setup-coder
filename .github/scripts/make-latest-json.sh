#!/usr/bin/env bash
# make-latest-json.sh —— 生成 latest.json(Release 产物索引)。
#
# 本脚本只服务 CI(release.yml),放 .github/ 而非 scripts/;
# scripts/ 是面向用户的 one-liner 脚本目录(工单 #5),不要混入。
#
# 用法:make-latest-json.sh <tag> <产物目录> <输出文件>
#   <产物目录> 中为按 setup-coder-<target>[.exe] 命名好的四平台二进制(平铺)。
#
# latest.json 结构(#5 one-liner 脚本与镜像分发的解析契约):
# {
#   "version": "0.1.0",          // 去掉 v 前缀的版本号
#   "tag": "v0.1.0",
#   "download_base": "https://github.com/<owner>/<repo>/releases/download/<tag>/",
#   "platforms": {
#     "<target>": { "path": "setup-coder-<target>[.exe]", "url": "<完整下载 URL>", "sha256": "<hex>" }
#   },
#   "install": { "sh": "install.sh" | null, "ps1": "install.ps1" | null }
# }
#
# 所有产物平铺在 Release 根目录,因此:
#   GitHub 完整下载 URL = download_base + path
#   OSS/Gitee 镜像整目录拷贝后   = <镜像根 URL> + path(同构)
set -euo pipefail

tag="$1"
dir="$2"
out="$3"
version="${tag#v}"
repo="${GITHUB_REPOSITORY:-atoz-project/setup-coder}"

# tag 与 Cargo.toml 版本对齐检查(rc/beta 等预发布后缀不参与比较)
crate_version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
if [ "${version%%-*}" != "$crate_version" ]; then
  echo "::error::tag $tag 与 Cargo.toml 版本 $crate_version 不一致,请先 bump 版本" >&2
  exit 1
fi

platforms='{}'
count=0
download_base="https://github.com/$repo/releases/download/$tag/"
for f in "$dir"/setup-coder-*; do
  [ -f "$f" ] || continue
  base=$(basename "$f")
  target="${base#setup-coder-}"
  target="${target%.exe}"
  sum=$(sha256sum "$f" | cut -d' ' -f1)
  platforms=$(jq --arg t "$target" --arg p "$base" --arg u "$download_base$base" --arg s "$sum" \
    '. + {($t): {path: $p, url: $u, sha256: $s}}' <<<"$platforms")
  count=$((count + 1))
done

if [ "$count" -ne 4 ]; then
  echo "::error::预期 4 个平台产物,实际 $count 个($dir/setup-coder-*)" >&2
  exit 1
fi

# one-liner 脚本属 #5:存在则记录文件名,不存在为 null(结构预留)
sh_val=null;  [ -f scripts/install.sh ]  && sh_val='"install.sh"'
ps1_val=null; [ -f scripts/install.ps1 ] && ps1_val='"install.ps1"'

jq -n \
  --arg version "$version" \
  --arg tag "$tag" \
  --arg base "$download_base" \
  --argjson platforms "$platforms" \
  --argjson sh "$sh_val" \
  --argjson ps1 "$ps1_val" \
  '{
     version: $version,
     tag: $tag,
     download_base: $base,
     platforms: $platforms,
     install: { sh: $sh, ps1: $ps1 }
   }' > "$out"

echo "已生成 $out:"
cat "$out"
