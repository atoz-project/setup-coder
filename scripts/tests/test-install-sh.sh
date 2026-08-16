#!/bin/sh
# test-install-sh.sh —— install.sh 可测逻辑的最小测试(平台探测 / 字段提取 / 源链拼接)。
# 用法:sh scripts/tests/test-install-sh.sh(需在仓库根目录;POSIX sh,dash 兼容)

set -u
cd "$(dirname "$0")/../.."

SETUP_CODER_TEST=1
. ./scripts/install.sh

fails=0
check() { # check <用例名> <期望> <实际>
  if [ "$2" = "$3" ]; then
    printf 'ok   %s\n' "$1"
  else
    printf 'FAIL %s\n  期望: %s\n  实际: %s\n' "$1" "$2" "$3"
    fails=$((fails + 1))
  fi
}

# --- 平台探测 ---
check "mac arm64"   "mac-arm64"  "$(_target_from_uname Darwin arm64)"
check "mac intel"   "mac-x64"    "$(_target_from_uname Darwin x86_64)"
check "linux x64"   "linux-x64"  "$(_target_from_uname Linux x86_64)"
_target_from_uname Linux aarch64 >/dev/null 2>&1
check "linux arm 不支持(返回非 0)" "1" "$?"
_target_from_uname FreeBSD x86_64 >/dev/null 2>&1
check "FreeBSD 不支持(返回非 0)" "1" "$?"

# --- latest.json 字段提取(结构即契约,样例同真实 Release 形态)---
fixture='
{
  "version": "0.1.0",
  "tag": "v0.1.0",
  "download_base": "https://github.com/atoz-project/setup-coder/releases/download/v0.1.0/",
  "platforms": {
    "linux-x64": {
      "path": "setup-coder-linux-x64",
      "url": "https://github.com/atoz-project/setup-coder/releases/download/v0.1.0/setup-coder-linux-x64",
      "sha256": "aaaa"
    },
    "mac-arm64": {
      "path": "setup-coder-mac-arm64",
      "url": "https://github.com/atoz-project/setup-coder/releases/download/v0.1.0/setup-coder-mac-arm64",
      "sha256": "bbbb"
    },
    "win-x64": {
      "path": "setup-coder-win-x64.exe",
      "url": "https://github.com/atoz-project/setup-coder/releases/download/v0.1.0/setup-coder-win-x64.exe",
      "sha256": "cccc"
    }
  },
  "install": { "sh": "install.sh", "ps1": null }
}'

block=$(printf '%s' "$fixture" | _json_block mac-arm64)
check "提取 path(mac-arm64)"   "setup-coder-mac-arm64" "$(printf '%s\n' "$block" | _json_field path)"
check "提取 sha256(mac-arm64)" "bbbb"                  "$(printf '%s\n' "$block" | _json_field sha256)"
block=$(printf '%s' "$fixture" | _json_block win-x64)
check "提取 path(win-x64)"     "setup-coder-win-x64.exe" "$(printf '%s\n' "$block" | _json_field path)"
check "提取 sha256(win-x64)"   "cccc"                    "$(printf '%s\n' "$block" | _json_field sha256)"

# --- 源链拼接:空源跳过、尾部斜杠归一、GitHub 前缀逐个试 + 直连兜底 ---
OSS_ROOT="" GITEE_ROOT="" GITHUB_PREFIXES="https://p1/ https://p2/" GITHUB_REPO="o/r"
got=$(latest_json_urls)
want='https://p1/https://github.com/o/r/releases/latest/download/latest.json
https://p2/https://github.com/o/r/releases/latest/download/latest.json
https://github.com/o/r/releases/latest/download/latest.json'
check "latest.json 链(空镜像跳过)" "$want" "$got"

OSS_ROOT="https://oss.example.com/" GITEE_ROOT="https://gitee.example.com/mirror" GITHUB_PREFIXES="" GITHUB_REPO="o/r"
got=$(binary_urls "setup-coder-mac-arm64" "https://github.com/o/r/x")
want='https://oss.example.com/setup-coder-mac-arm64
https://gitee.example.com/mirror/setup-coder-mac-arm64
https://github.com/o/r/x'
check "二进制链(镜像根归一 + 直连)" "$want" "$got"

if [ "$fails" -eq 0 ]; then
  echo "全部通过"
else
  echo "$fails 个用例失败" >&2
  exit 1
fi
