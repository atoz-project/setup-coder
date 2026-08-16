#!/bin/sh
# install.sh —— setup-coder 的 One-liner 自举脚本(macOS / Ubuntu)
#
# 用法(One-liner,见 CONTEXT.md):
#   curl -fsSL <本脚本地址> | sh
#
# 职责(脚本只做"下载二进制并转交",逻辑全在二进制里):
#   探测平台/架构 → 取 latest.json → 按容错链下载对应二进制
#   → sha256 校验 → 放入 ~/.setup-coder/bin/ → 执行 setup-coder install
#
# ===== 下载容错链配置(维护者只改这里)=====
#
# 容错链顺序:OSS → Gitee → GitHub(加速前缀依次试,最后直连)。
# OSS / Gitee 镜像根 URL 留空 = 跳过该源;填上域名即生效(需以 / 结尾,脚本会自动补)。
# 镜像内容 = GitHub Release 根目录整目录拷贝(平铺),因此:
#   镜像上的 latest.json = <镜像根> + latest.json
#   镜像上的二进制       = <镜像根> + <latest.json 里的 path 字段>
OSS_ROOT=""
GITEE_ROOT=""
# GitHub 加速前缀(ghproxy 类),按顺序尝试;全部失败后自动直连 GitHub。
# 这类公共服务时效性强,失效时换一个即可,格式:<前缀> + 完整 GitHub URL。
GITHUB_PREFIXES="https://ghfast.top/ https://gh-proxy.com/ https://ghproxy.net/"

GITHUB_REPO="atoz-project/setup-coder"
# ===================================

BIN_NAME="setup-coder"

say()  { printf '%s\n' "$*"; }
step() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m提醒:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m失败:\033[0m %s\n' "$*" >&2; exit 1; }

# --- 平台探测(拆成纯函数,便于测试;见 scripts/tests/)---

# _target_from_uname <uname -s> <uname -m> → 输出 target,不支持则返回非 0
_target_from_uname() {
  case "$1" in
    Darwin)
      case "$2" in
        arm64|aarch64) echo "mac-arm64" ;;
        x86_64)        echo "mac-x64" ;;
        *) return 1 ;;
      esac ;;
    Linux)
      case "$2" in
        x86_64|amd64) echo "linux-x64" ;;
        *) return 1 ;;
      esac ;;
    *) return 1 ;;
  esac
}

detect_target() {
  os=$(uname -s 2>/dev/null) || die "无法识别操作系统(uname 不可用)。"
  arch=$(uname -m 2>/dev/null) || die "无法识别 CPU 架构(uname 不可用)。"
  _target_from_uname "$os" "$arch" || die "暂不支持的平台:$os/$arch。
目前支持:macOS(Apple 芯片 / Intel)和 Ubuntu x64。Windows 请用 install.ps1。"
}

# --- latest.json 字段提取(结构契约见 .github/scripts/make-latest-json.sh 文件头)---
# 不依赖 jq/python3:结构固定,用 sed 按目标平台的键名截取区块再取字段。

# _json_block <目标平台> —— 从 stdin 的 latest.json 中截出该平台的 { ... } 区块
_json_block() {
  sed -n "/\"$1\"[[:space:]]*:[[:space:]]*{/,/^[[:space:]]*}/p"
}

# _json_field <字段名> —— 从 stdin 的区块中取出 "字段": "值" 的值
_json_field() {
  sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\\1/p" | sed -n '1p'
}

# --- 下载源链 ---

# _normalize_root <根 URL> —— 去掉尾部 /,便于统一拼接;空则原样返回
_normalize_root() {
  [ -n "$1" ] && echo "${1%/}"
}

# _mirror_urls <文件名> —— 输出该文件在各 Mirror(镜像源)上的候选 URL;空源自动跳过
_mirror_urls() {
  oss=$(_normalize_root "$OSS_ROOT")
  gitee=$(_normalize_root "$GITEE_ROOT")
  [ -n "$oss" ]   && echo "$oss/$1"
  [ -n "$gitee" ] && echo "$gitee/$1"
}

# latest_json_urls —— 输出 latest.json 的候选 URL,每行一个,按容错链顺序
latest_json_urls() {
  _mirror_urls latest.json
  gh_latest="https://github.com/$GITHUB_REPO/releases/latest/download/latest.json"
  for p in $GITHUB_PREFIXES; do
    echo "$p$gh_latest"
  done
  echo "$gh_latest"
}

# binary_urls <path 字段> <url 字段> —— 输出二进制的候选 URL,每行一个,按容错链顺序
binary_urls() {
  _mirror_urls "$1"
  for p in $GITHUB_PREFIXES; do
    echo "$p$2"
  done
  echo "$2"
}

# --- 下载与校验 ---

have() { command -v "$1" >/dev/null 2>&1; }

# fetch <url> <输出文件> —— 成功返回 0;优先 curl,缺了用 wget
fetch() {
  if have curl; then
    curl -fsSL --connect-timeout 10 --retry 0 -o "$2" "$1" 2>/dev/null
  elif have wget; then
    wget -q -T 15 -O "$2" "$1" 2>/dev/null
  else
    die "找不到 curl 或 wget,无法下载。请先安装 curl(Ubuntu:sudo apt install curl)后重试。"
  fi
}

# sha256_of <文件> —— 输出 sha256(hex);两个工具都没有则报错退出
sha256_of() {
  if have shasum; then
    shasum -a 256 "$1" | cut -d' ' -f1
  elif have sha256sum; then
    sha256sum "$1" | cut -d' ' -f1
  else
    die "找不到 shasum 或 sha256sum,无法校验文件完整性。请安装 coreutils 后重试。"
  fi
}

# --- 主流程 ---

main() {
  say "setup-coder 一键安装"
  say "===================="

  step "第 1 步:识别你的电脑平台和 CPU 架构"
  target=$(detect_target)
  say "识别结果:$target"

  step "第 2 步:获取最新版本信息(latest.json)"
  say "会依次尝试多个下载源,某个源连不上会自动换下一个,请稍等。"
  tmpdir=$(mktemp -d 2>/dev/null) || die "无法创建临时目录,请检查磁盘空间。"
  trap 'rm -rf "$tmpdir"' EXIT
  trap 'exit 130' INT TERM

  json_file="$tmpdir/latest.json"
  json_ok=""
  for url in $(latest_json_urls); do
    say "尝试:$url"
    if fetch "$url" "$json_file" && [ -s "$json_file" ]; then
      say "获取成功。"
      json_ok=1
      break
    fi
    warn "这个源连不上,换下一个……"
  done
  [ -n "$json_ok" ] || die "所有下载源都连不上 latest.json。
可能原因:当前网络完全无法访问 GitHub 及镜像。
建议:检查网络后重试;若反复失败,请到 https://github.com/$GITHUB_REPO/issues 反馈。"

  block=$(printf '%s\n' "$(cat "$json_file")" | _json_block "$target")
  path=$(printf '%s\n' "$block" | _json_field path)
  url_field=$(printf '%s\n' "$block" | _json_field url)
  want_sha=$(printf '%s\n' "$block" | _json_field sha256)
  [ -n "$path" ] && [ -n "$url_field" ] && [ -n "$want_sha" ] \
    || die "latest.json 里找不到 $target 的下载信息,可能是版本索引损坏,请向项目反馈。"

  step "第 3 步:下载 setup-coder 二进制($path)"
  bin_tmp="$tmpdir/$path"
  bin_ok=""
  for url in $(binary_urls "$path" "$url_field"); do
    say "尝试:$url"
    if fetch "$url" "$bin_tmp" && [ -s "$bin_tmp" ]; then
      say "下载完成,正在校验文件完整性(sha256)……"
      got_sha=$(sha256_of "$bin_tmp")
      if [ "$got_sha" = "$want_sha" ]; then
        say "校验通过。"
        bin_ok=1
        break
      fi
      warn "这个源下载的文件校验不一致(可能传输损坏),换下一个源重试……"
    else
      warn "这个源连不上,换下一个……"
    fi
  done
  [ -n "$bin_ok" ] || die "所有下载源都拿不到完好的二进制。
可能原因:网络不稳定导致文件反复损坏,或镜像内容过期。
建议:稍后重试;若反复失败,请到 https://github.com/$GITHUB_REPO/issues 反馈。"

  step "第 4 步:安装到 ~/.setup-coder/bin/"
  bin_dir="$HOME/.setup-coder/bin"
  mkdir -p "$bin_dir" || die "无法创建目录 $bin_dir,请检查权限。"
  cp "$bin_tmp" "$bin_dir/$BIN_NAME" || die "无法写入 $bin_dir/$BIN_NAME,请检查权限与磁盘空间。"
  chmod +x "$bin_dir/$BIN_NAME" || die "无法给 $bin_dir/$BIN_NAME 加执行权限。"
  say "已放好:$bin_dir/$BIN_NAME"

  step "第 5 步:启动安装(setup-coder install)"
  say "接下来由 setup-coder 自动装好前置依赖(Node.js、git)和各 Tool,全程无需输入。"
  if "$bin_dir/$BIN_NAME" install; then
    say ""
    say "全部完成!重新打开一个终端窗口即可使用。"
  else
    die "二进制已就位,但 setup-coder install 执行失败。
你可以稍后手动重跑:\"$bin_dir/$BIN_NAME\" install
若反复失败,请到 https://github.com/$GITHUB_REPO/issues 反馈。"
  fi
}

# 测试模式下只加载函数不执行(见 scripts/tests/test-install-sh.sh)
if [ "${SETUP_CODER_TEST:-}" != "1" ]; then
  main "$@"
fi
