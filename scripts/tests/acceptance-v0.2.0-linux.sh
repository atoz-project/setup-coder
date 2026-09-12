#!/usr/bin/env bash

# =============================================================================
# accept-v0.2.0-linux.sh — 真机验收脚本(Ubuntu 24.04,bash)
# setup-coder v0.2.0「Node 前置复用,不劫持环境」特性
#
# 运行方式:ECS RunCommand(RunShellScript)以【目标普通用户】身份执行,
#   绝不用 root(家目录/rc 判定全部基于 $HOME;root 会弄脏 /root)。
#   需要 sudo 的唯一场景:机器无 git 时 setup-coder 会 `sudo apt-get install -y git`,
#   建议给该用户 NOPASSWD sudo,或预装 git。
#
# 流程(幂等、自清理):
#   PRE 快照 → 下载 v0.2.0 二进制 → install → 断言(1-7) → uninstall --yes
#   → 断言回滚(8-9) → 若 setup-coder 代装了 fnm,则按 uninstall 自己打印的
#   「手工移除步骤」清理代装 fnm 资产,恢复到 PRE 状态。
#   任何 FAIL 都会触发同样的清理(EXIT trap),退出码非 0。
#
# 输出协议:每个检查点一行 `ACCEPT <n> PASS|FAIL: <描述>`(方便 grep);
#   `EXPECT ...` / `NOTE ...` 为决策依据与说明;结尾打印汇总,任一 FAIL → exit 1。
# =============================================================================
set -u
export LC_ALL=C.UTF-8 LANG=C.UTF-8   # 保证 grep 按字节匹配,中文行也能稳定查找

# ----------------------------- 常量(与 v0.2.0 代码逐字对齐) -----------------
VER="v0.2.0"
ASSET="setup-coder-linux-x64"
DL_URL="https://github.com/atoz-project/setup-coder/releases/download/${VER}/${ASSET}"
# GitHub 加速前缀(与 scripts/install.sh 同一条容错链;逐个试,最后直连)
GH_PREFIXES="https://ghfast.top/ https://gh-proxy.com/ https://ghproxy.net/"

PREFIX="$HOME/.setup-coder"          # 私有前缀根
BIN_DIR="$PREFIX/bin"                # 唯一进 PATH 的目录
RC_FILES="$HOME/.bashrc $HOME/.zshrc"          # src/platform/linux.rs RC_FILES
FLOOR_ALL="22.19.0"                  # 全量工具(codex/claude/pi)的 Node 下限
FNM_DIR="$HOME/.local/share/fnm"     # unix fnm 默认数据目录(src/platform/mod.rs)
FNM_NODE_BASE="$FNM_DIR/node-versions"          # 快照其下内容以识别新增版本
FNM_HOOK_LINE='eval "$("'"$FNM_DIR"'/fnm" env --use-on-cd)"  # setup-coder fnm'   # fnm_hook_line(fnm_exe),绝对路径不依赖 PATH
PATH_LINE="export PATH=\"$BIN_DIR:\$PATH\"  # setup-coder"           # shell_rc_export_line()
NVM_DIR_DEFAULT="$HOME/.nvm"

PASS=0; FAIL=0
declare -a RESULTS=()
say()  { printf '%s\n' "$*"; }
note() { say "NOTE $*"; }
ok()   { PASS=$((PASS+1)); RESULTS+=("ACCEPT $1 PASS: $2"); say "ACCEPT $1 PASS: $2"; }
bad()  { FAIL=$((FAIL+1)); RESULTS+=("ACCEPT $1 FAIL: $2"); say "ACCEPT $1 FAIL: $2"; }

# 版本比较:a >= b(参数均为 x.y.z;补齐缺省段为 0)
ver_ge() {
  local a="$1" b="$2" IFS=.
  local -a A=($a) B=($b) i
  for i in 0 1 2; do
    local x=${A[$i]:-0} y=${B[$i]:-0}
    ((10#$x > 10#$y)) && return 0
    ((10#$x < 10#$y)) && return 1
  done
  return 0
}

# ------------------------------- 幂等清理器 ----------------------------------

# ------------------------- PRE 状态变量(先默认,快照阶段再填) -----------------
# cleanup 在任何失败路径都要安全运行,故全部变量先给默认值
SNAP_DIR=""
PRE_HAD_NVM=0; PRE_NVM_CURRENT=""
PRE_HAD_FNM=0; PRE_HAD_FNM_DIR=0; PRE_FNM_VERSIONS=""; PRE_FNM_DEFAULT=""
PRE_HAD_GIT=0; PRE_HAD_NPMRC=0; PRE_HAD_PREFIX=0
EXPECT_SOURCE=""; EXPECT_FNM_HOOK=0

# 目标:无论脚本在哪一步失败,都把机器恢复到 PRE 快照(或明确报告无法恢复的残留)。
cleanup() {
  set +e
  # 1) setup-coder 自己的卸载(清单驱动的精确回滚)
  if [ -d "$PREFIX" ]; then
    if [ -x "$PREFIX/bin/setup-coder" ]; then
      "$PREFIX/bin/setup-coder" uninstall --yes >/dev/null 2>&1
    fi
    [ -d "$PREFIX" ] && rm -rf "$PREFIX"
  fi
  # 2) 若 install 是「代装 fnm」分支(快照里没有 fnm 目录,现在有了):
  #    uninstall 按设计【保留】fnm;此处按它打印的手工步骤恢复 PRE 状态。
  if [ "${PRE_HAD_FNM_DIR:-0}" = "0" ] && [ -d "$FNM_DIR" ]; then
    rm -rf "$FNM_DIR"
  fi
  # 若机器本来就有 fnm,仅删除 setup-coder 新增的版本目录,保留原有全部
  if [ "${PRE_HAD_FNM_DIR:-0}" = "1" ] && [ -d "$FNM_NODE_BASE" ]; then
    local d b
    for d in "$FNM_NODE_BASE"/v*; do
      [ -d "$d" ] || continue
      b=$(basename "$d")
      case " ${PRE_FNM_VERSIONS:-} " in *" $b "*) : ;; *) rm -rf "$d" ;; esac
    done
    # default 若被 install 改指到新版本,恢复(尽量;无对应记录时仅能报警)
    if [ -n "${PRE_FNM_DEFAULT:-}" ] && [ -x "$FNM_DIR/fnm" ]; then
      "$FNM_DIR/fnm" default "$PRE_FNM_DEFAULT" >/dev/null 2>&1
    fi
  fi
  # 3) rc 文件:回滚按 state.json 精确删行后会【重写】文件(尾部空白行被裁掉、
  #    末尾保证恰好一个换行),与 PRE 快照逐字节比较有假阳性。正确做法:
  #    从【当前内容】里删掉 setup-coder 注入行(幂等),再与 PRE 快照内容比较
  #    (命令替换天然去掉尾部换行/空行 = 同一规范化)。仍不一致则以快照为准恢复。
  local f snap cur norm_snap norm_cur
  for f in $RC_FILES; do
    snap="$SNAP_DIR/$(basename "$f")"
    if [ -f "$snap" ]; then
      if [ -f "$f" ]; then
        cur=$(grep -vxF "$PATH_LINE" "$f" | grep -vxF "$FNM_HOOK_LINE" || true)
        norm_snap=$(cat "$snap")
        norm_cur=$cur
        if [ "$norm_cur" != "$norm_snap" ]; then
          cp -f "$snap" "$f"
        fi
      else
        cp -f "$snap" "$f"   # 文件被删了,恢复
      fi
    elif [ -f "$f" ]; then
      # PRE 不存在此 rc:删掉注入行;若删除后为空则删文件
      cur=$(grep -vxF "$PATH_LINE" "$f" 2>/dev/null | grep -vxF "$FNM_HOOK_LINE" || true)
      if [ -z "$(printf '%s' "$cur" | tr -d '[:space:]')" ]; then
        rm -f "$f"
      else
        printf '%s\n' "$cur" > "$f"
      fi
    fi
  done
  if [ -n "${PRE_NVM_CURRENT:-}" ] && [ -d "$NVM_DIR_DEFAULT/alias" ]; then
    printf '%s\n' "$PRE_NVM_CURRENT" > "$NVM_DIR_DEFAULT/alias/current" 2>/dev/null
  fi
  rm -rf "$SNAP_DIR"
  say "CLEANUP done (machine restored to pre-run state as far as possible)"
}
trap cleanup EXIT

# ------------------------------- PRE:环境基线 --------------------------------
say "=================================================================="
say " setup-coder $VER 验收(Linux)— $(date -Is) — user=$(id -un) home=$HOME"
say "=================================================================="

[ -f /etc/os-release ] && . /etc/os-release 2>/dev/null
note "os=${PRETTY_NAME:-unknown}"

SNAP_DIR=$(mktemp -d /tmp/setup-coder-accept.XXXXXX) || { say "FATAL: mktemp failed"; exit 2; }

# git 是硬前置(无 git 且无 sudo 时 install 会直接失败)
if command -v git >/dev/null 2>&1; then
  note "git present: $(command -v git)"
  PRE_HAD_GIT=1
else
  note "git NOT on PATH — install 将尝试 sudo apt-get 安装(属于离开共享机前需人工注意的系统改动)"
  PRE_HAD_GIT=0
  if ! sudo -n true 2>/dev/null; then
    say "FATAL: 无 git 且 sudo 需要密码,install 必然失败。请预装 git 或配置 NOPASSWD sudo。"
    exit 2
  fi
fi

# --- 探测 Node 三来源(镜像 src/platform/mod.rs detect_node_facts_impl) ---
PRE_NODE_PATH=""; PRE_NODE_VER=""
if command -v node >/dev/null 2>&1; then
  PRE_NODE_PATH=$(command -v node)
  PRE_NODE_VER=$(node --version 2>/dev/null | sed 's/^v//')
fi

# --- nvm 探测(与 detect_nvm_unix 同一口径:~/.nvm 目录存在,或 rc 里有 nvm 行) ---
_rc_any_nvm=0
for f in $RC_FILES; do
  [ -f "$f" ] && grep -q 'nvm' "$f" 2>/dev/null && _rc_any_nvm=1
done
if [ -d "$NVM_DIR_DEFAULT" ] || [ "$_rc_any_nvm" = "1" ]; then
  PRE_HAD_NVM=1
  [ -f "$NVM_DIR_DEFAULT/alias/current" ] && PRE_NVM_CURRENT=$(cat "$NVM_DIR_DEFAULT/alias/current")
fi

# --- fnm 探测(与 detect_fnm_unix 同一口径:PATH 上有 fnm,或默认数据目录存在) ---
if command -v fnm >/dev/null 2>&1 || [ -d "$FNM_DIR" ]; then
  PRE_HAD_FNM=1
fi
if [ -d "$FNM_DIR" ]; then
  PRE_HAD_FNM_DIR=1
  [ -d "$FNM_NODE_BASE" ] && PRE_FNM_VERSIONS=$(ls "$FNM_NODE_BASE" 2>/dev/null | tr '\n' ' ')
  # 记录当前 default 版本(仅当数据目录里【已存在】该版本目录——只记录真实存在的,
  # 避免把 setup-coder 之后装的新版本误当 PRE default 去"恢复"一个不存在的别名)
  _cand=""
  if command -v fnm >/dev/null 2>&1; then
    _cand=$(fnm list 2>/dev/null | grep -o 'v[0-9][0-9.]*' | sort -V | tail -1)
  elif [ -x "$FNM_DIR/fnm" ]; then
    _cand=$("$FNM_DIR/fnm" list 2>/dev/null | grep -o 'v[0-9][0-9.]*' | sort -V | tail -1)
  fi
  if [ -n "$_cand" ] && [ -d "$FNM_NODE_BASE/$_cand" ]; then PRE_FNM_DEFAULT="$_cand"; fi
fi

# --- decide() 期望值推导(与 src/node_plan.rs 锁定优先级一致) ---
# 注意:detect_node_facts 中的「裸 Node」判定是 `PATH 上能找到 node`——
# nvm/fnm 已激活的 shell 里 `command -v node` 也会命中,此时实现同样走
# ReuseBareNode。本脚本与实现使用同一探测口径,故期望一致。
EXPECT_SOURCE=""
if [ -n "$PRE_NODE_VER" ] && ver_ge "$PRE_NODE_VER" "$FLOOR_ALL"; then
  EXPECT_SOURCE="user_bare"
elif [ "$PRE_HAD_NVM" = "1" ]; then
  EXPECT_SOURCE="user_nvm"
elif [ "$PRE_HAD_FNM" = "1" ]; then
  EXPECT_SOURCE="user_fnm"
else
  EXPECT_SOURCE="user_fnm"   # InstallFnm 落账也是 user_fnm;靠 FnmHook 注入记录区分
fi
EXPECT_FNM_HOOK=0
if [ -z "$PRE_NODE_VER" ] || ! ver_ge "$PRE_NODE_VER" "$FLOOR_ALL"; then
  if [ "$PRE_HAD_NVM" = "0" ] && [ "$PRE_HAD_FNM" = "0" ]; then EXPECT_FNM_HOOK=1; fi
fi
say "EXPECT node.source=$EXPECT_SOURCE (bare=${PRE_NODE_VER:-none}@${PRE_NODE_PATH:-none} nvm=$PRE_HAD_NVM fnm=$PRE_HAD_FNM; floor=$FLOOR_ALL)"
say "EXPECT fnm_hook_injected=$EXPECT_FNM_HOOK (1 = setup-coder 代装 fnm 分支)"

# --- 快照(恢复用) ---
for f in $RC_FILES; do
  [ -f "$f" ] && cp -p "$f" "$SNAP_DIR/$(basename "$f")"
done
[ -f "$HOME/.npmrc" ] && { cp -p "$HOME/.npmrc" "$SNAP_DIR/user-npmrc"; PRE_HAD_NPMRC=1; } || PRE_HAD_NPMRC=0
[ -d "$PREFIX" ] && PRE_HAD_PREFIX=1 || PRE_HAD_PREFIX=0
note "snapshot dir: $SNAP_DIR (prefix_existed=$PRE_HAD_PREFIX npmrc_existed=$PRE_HAD_NPMRC)"
if [ "$PRE_HAD_PREFIX" = "1" ]; then
  say "FATAL: $PREFIX 已存在——本机已有 setup-coder 安装,验收会干扰它。请先手工卸载再跑。"
  exit 2
fi

# ------------------------------- 下载二进制 ----------------------------------
WORK_DIR=$(mktemp -d /tmp/setup-coder-bin.XXXXXX)
BIN="$WORK_DIR/setup-coder"
dl_ok=0
for pre in $GH_PREFIXES ""; do
  url="${pre}${DL_URL}"
  note "trying download: $url"
  if curl -fsSL --connect-timeout 15 --max-time 300 -o "$BIN" "$url" 2>/dev/null; then dl_ok=1; break; fi
done
if [ "$dl_ok" != "1" ]; then
  bad 1 "下载 $ASSET 失败(GitHub 直连与加速前缀均不可达)"
  say "ACCEPT-SUMMARY pass=$PASS fail=$FAIL (aborted at download)"
  exit 1
fi
chmod +x "$BIN"
dl_bytes=$(stat -c%s "$BIN" 2>/dev/null || echo 0)
[ "$dl_bytes" -gt 1000000 ] && note "downloaded ${dl_bytes} bytes" || note "WARNING: binary suspiciously small (${dl_bytes}B)"

# ------------------------------- ACCEPT 1:install 端到端 ----------------------
install_log="$SNAP_DIR/install.log"
install_rc=0
"$BIN" install >"$install_log" 2>&1 || install_rc=$?
if [ "$install_rc" = "0" ]; then
  ok 1 "install 端到端成功(exit 0;工具经前缀内 npm 安装并冒烟)"
else
  bad 1 "install 失败(exit $install_rc;见下方日志尾)"
  tail -n 25 "$install_log" | sed 's/^/  INSTALL-LOG /'
fi
# 三个 shim 都应生成(codex / claude / pi)
for t in codex claude pi; do
  if [ ! -x "$BIN_DIR/$t" ]; then bad 1 "缺少 shim:$BIN_DIR/$t"; fi
done

# -------------------- ACCEPT 2:Node 来源与探测前状态一致 ----------------------
STATE="$PREFIX/state.json"
state_src=""; state_ver=""; state_exe=""
if [ -f "$STATE" ]; then
  state_src=$(sed -n 's/.*"source"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$STATE" | head -1)
  state_ver=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\(v[0-9.]*\)".*/\1/p' "$STATE" | head -1)
  state_exe=$(sed -n 's/.*"exe"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$STATE" | head -1)
fi
if [ "$state_src" = "$EXPECT_SOURCE" ]; then
  ok 2 "Node 来源与决策一致:state.json source=$state_src(期望 $EXPECT_SOURCE)"
else
  bad 2 "Node 来源不符:state.json source=${state_src:-missing}(期望 $EXPECT_SOURCE)"
fi

# -------------------- ACCEPT 3:去劫持证明(PATH 抹掉 node 仍可跑) ------------
# 用 env -i 构造最小 PATH(不含任何 node/nvm/fnm 目录),shim 必须仍报版本。
dehijack_ok=1
for t in codex claude pi; do
  shim="$BIN_DIR/$t"
  [ -x "$shim" ] || { dehijack_ok=0; continue; }
  out=$(env -i PATH=/usr/bin:/bin HOME="$HOME" "$shim" --version 2>&1)
  rc=$?
  if [ $rc -ne 0 ] || [ -z "$out" ]; then
    dehijack_ok=0
    note "de-hijack run failed: $t rc=$rc out=$out"
  fi
done
# 代装 fnm 分支:钩子行本身须在干净 PATH 下可用(v0.3.0 回归:旧钩子假定 fnm
# 在 PATH,新开终端必报 "Command 'fnm' not found" 且 node 不出现)
if [ "$EXPECT_FNM_HOOK" = "1" ]; then
  hook_out=$(env -i PATH=/usr/bin:/bin HOME="$HOME" bash -c "$FNM_HOOK_LINE; node --version" 2>&1)
  if [ $? -ne 0 ] || [ -z "$hook_out" ]; then
    dehijack_ok=0
    note "干净 PATH 下跑 fnm 钩子拿不到 node: $hook_out"
  fi
fi
if [ $dehijack_ok = 1 ]; then
  ok 3 "去劫持:env -i PATH=/usr/bin:/bin 下 codex/claude/pi --version 均 exit 0 且有输出"
else
  bad 3 "去劫持:最小 PATH 下有 shim 不能正常启动(见 NOTE)"
fi

# -------------------- ACCEPT 4:前缀内无 node/npm;PATH 只加 bin/ --------------
n4=1
# 4a: bin/ 下无 node/npm 可执行文件
for x in node node.exe npm npm.cmd npx npx.cmd; do
  [ -e "$BIN_DIR/$x" ] && { n4=0; note "prefix bin 含违禁文件:$x"; }
done
# 4b: 整个前缀内不允许出现 node 发行布局(Node 永不落前缀;cache 里允许有 fnm 压缩包)
if find "$PREFIX" -path "$PREFIX/cache" -prune -o -type f -name 'node' -print 2>/dev/null | grep -q .; then
  n4=0; note "prefix 内(除 cache)发现 node 可执行文件"
fi
# 4c: rc 注入行恰好一条、内容逐字等于 PATH_LINE,且不含任何 node 目录
n4c=1
for f in $RC_FILES; do
  cnt=$(grep -cF '# setup-coder' "$f" 2>/dev/null); cnt=${cnt:-0}
  : "$cnt"   # 引用以防未使用告警;防重复靠下面 PATH_LINE 精确计数
  hits=$(grep -nF "$PATH_LINE" "$f" 2>/dev/null | wc -l)
  if [ "$hits" -gt 1 ]; then n4c=0; note "$f 中 PATH 注入行重复($hits 次)"; fi
  # 注入行不应指向 node/nvm/fnm 目录
  if grep -F '# setup-coder' "$f" | grep -E 'node|nvm|fnm' | grep -vF '# setup-coder fnm' >/dev/null 2>&1; then
    n4c=0; note "$f 中存在指向 node 目录的 setup-coder PATH 注入"
  fi
done
if [ $n4 = 1 ] && [ $n4c = 1 ]; then
  ok 4 "前缀内无 node/npm;rc 中唯一的 PATH 注入是 bin/(无 node 目录入 PATH)"
else
  bad 4 "前缀发现 node/npm,或 PATH 注入异常(见 NOTE)"
fi

# -------------------- ACCEPT 5:state.json v2 三值来源 ------------------------
if [ -f "$STATE" ] && grep -q '"version"[[:space:]]*:[[:space:]]*2' "$STATE"; then
  case "$state_src" in
    user_bare|user_nvm|user_fnm)
      ok 5 "state.json v2:version=2 且 node.source=$state_src(三值枚举,无 prefix 值)";;
    *)
      bad 5 "state.json node.source=${state_src:-missing} 非三值枚举";;
  esac
else
  bad 5 "state.json 缺失或 version≠2"
fi

# -------------------- ACCEPT 6:用户 ~/.npmrc 未被触碰 -------------------------
n6=1
if [ "$PRE_HAD_NPMRC" = "1" ]; then
  if ! cmp -s "$SNAP_DIR/user-npmrc" "$HOME/.npmrc" 2>/dev/null; then
    n6=0; note "用户 ~/.npmrc 内容被改动"
  fi
else
  [ -e "$HOME/.npmrc" ] && { n6=0; note "install 新建了用户 ~/.npmrc(不应存在)"; }
fi
# 顺带:前缀内 .npmrc 应存在且指向 npmmirror(契约:registry 只写前缀内)
if ! grep -q 'registry=https://registry.npmmirror.com' "$PREFIX/.npmrc" 2>/dev/null; then
  n6=0; note "前缀内 .npmrc 缺失或未指向 npmmirror"
fi
if [ $n6 = 1 ]; then
  ok 6 "用户 ~/.npmrc 未改动;registry 配置只写在前缀内 .npmrc"
else
  bad 6 "npm 配置越界或前缀 .npmrc 异常(见 NOTE)"
fi

# -------------------- ACCEPT 7:幂等重跑 --------------------------------------
sha_state_before=$(sha256sum "$STATE" 2>/dev/null | cut -d' ' -f1)
rc_lines_before=""
for f in $RC_FILES; do
  [ -f "$f" ] && rc_lines_before="$rc_lines_before$f:$(grep -cF '# setup-coder' "$f" 2>/dev/null);"
done
mtime_fnm_dir=""
[ -e "$FNM_DIR/fnm" ] && mtime_fnm_dir=$(stat -c%Y "$FNM_DIR/fnm" 2>/dev/null)

rerun_log="$SNAP_DIR/rerun.log"
rerun_rc=0
"$BIN" install >"$rerun_log" 2>&1 || rerun_rc=$?
n7=1
[ $rerun_rc -ne 0 ] && { n7=0; note "重跑 install 失败(exit $rerun_rc)"; }
# 重跑后 rc 注入行数不得增加(无重复 hook/PATH 行)
for f in $RC_FILES; do
  [ -f "$f" ] || continue
  now=$(grep -cF '# setup-coder' "$f" 2>/dev/null); now=${now:-0}
  before=$(echo "$rc_lines_before" | tr ';' '\n' | sed -n "s|^$f:\([0-9]*\)|\1|p")
  [ "${now:-0}" -gt "${before:-0}" ] && { n7=0; note "$f 注入行数 $before→$now(重复注入)"; }
done
# 代装 fnm 分支:重跑不得重下 fnm(幂等复用 → fnm 二进制 mtime 不变)
if [ "$EXPECT_FNM_HOOK" = "1" ] && [ -n "$mtime_fnm_dir" ]; then
  mtime_now=$(stat -c%Y "$FNM_DIR/fnm" 2>/dev/null)
  [ "$mtime_now" != "$mtime_fnm_dir" ] && note "fnm 二进制 mtime 变化(可能重装;弱信号,仅记录)"
fi
# state.json 仍为合法 v2(重跑可能刷新 version 字段,但 source 不得变)
state_src2=$(sed -n 's/.*"source"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$STATE" 2>/dev/null | head -1)
[ "$state_src2" != "$EXPECT_SOURCE" ] && { n7=0; note "重跑后 source 变为 $state_src2"; }
if [ $n7 = 1 ]; then
  ok 7 "幂等重跑:exit 0,rc/profile 无重复注入行,source 不变"
else
  bad 7 "幂等重跑异常(见 NOTE)"
fi

# -------------------- ACCEPT 8:uninstall 回滚 ---------------------------------
uninstall_log="$SNAP_DIR/uninstall.log"
un_rc=0
"$BIN_DIR/setup-coder" uninstall --yes >"$uninstall_log" 2>&1 || un_rc=$?
n8=1
[ $un_rc -ne 0 ] && { n8=0; note "uninstall --yes 退出码 $un_rc"; }
# 8a: 前缀删除
if [ -d "$PREFIX" ]; then
  # unix 可删正在运行的文件,应整体删除
  n8=0; note "uninstall 后前缀仍存在:$PREFIX"
fi
# 8b: PATH 行回滚(rc 中不再有 setup-coder PATH 行)
for f in $RC_FILES; do
  [ -f "$f" ] || continue
  if grep -qxF "$PATH_LINE" "$f" 2>/dev/null; then n8=0; note "$f 中 PATH 注入行未回滚"; fi
done
# 8c: fnm 钩子行回滚(仅当代装分支注入过)
if [ "$EXPECT_FNM_HOOK" = "1" ]; then
  for f in $RC_FILES; do
    [ -f "$f" ] || continue
    if grep -qxF "$FNM_HOOK_LINE" "$f" 2>/dev/null; then n8=0; note "$f 中 fnm 钩子行未回滚"; fi
  done
fi
# 8d: 用户的 node/nvm/fnm 保留
if [ -n "$PRE_NODE_PATH" ] && [ ! -x "$PRE_NODE_PATH" ]; then n8=0; note "用户原有 node 消失:$PRE_NODE_PATH"; fi
if [ "$PRE_HAD_NVM" = "1" ] && [ ! -d "$NVM_DIR_DEFAULT" ] && [ -z "${NVM_DIR:-}" ]; then n8=0; note "用户 nvm 目录被删"; fi
if [ "$PRE_HAD_FNM_DIR" = "1" ] && [ ! -d "$FNM_DIR" ]; then n8=0; note "用户 fnm 目录被删"; fi
# 8e: 按来源的中文保留提示(uninstall 结尾打印)
hint_ok=0
case "$EXPECT_SOURCE" in
  user_bare) grep -q '你机器上原有的 Node' "$uninstall_log" && hint_ok=1 ;;
  user_nvm)  grep -q '你的 nvm 与其 Node' "$uninstall_log" && hint_ok=1 ;;
  user_fnm)
    if [ "$EXPECT_FNM_HOOK" = "1" ]; then
      grep -q 'setup-coder 为你安装了 fnm 与 Node' "$uninstall_log" && hint_ok=1
    else
      grep -q '你的 fnm 与其 Node' "$uninstall_log" && hint_ok=1
    fi ;;
esac
[ $hint_ok = 0 ] && { n8=0; note "中文保留提示缺失/不匹配(见 uninstall 输出)"; }
if [ $n8 = 1 ]; then
  ok 8 "uninstall:前缀已删,PATH/fnm 钩子按记录回滚,用户 Node 资产保留,中文提示正确"
else
  bad 8 "uninstall 回滚异常(见 NOTE)"
fi
tail -n 8 "$uninstall_log" | sed 's/^/  UNINSTALL-LOG /'

# -------------------- ACCEPT 9:与 PRE 快照零残留 ------------------------------
# 先清理 setup-coder 代装的 fnm 资产(uninstall 按设计保留;验收负责复原共享机)
restored_fnm_note=""
if [ "$EXPECT_FNM_HOOK" = "1" ] && [ "$PRE_HAD_FNM_DIR" = "0" ] && [ -d "$FNM_DIR" ]; then
  rm -rf "$FNM_DIR"
  restored_fnm_note="(已按 uninstall 提示手工删除代装 fnm:$FNM_DIR)"
fi
n9=1
# 9a: rc 文件:uninstall 按记录精确删行后会【重写】文件(尾部空白行被裁、末尾
#     恰好一个换行)。这是设计内行为,不算残留。断言「PRE 行全在 + 没有
#     setup-coder 注入行 + 没有新增实质行」(命令替换去尾部换行/空行规范化)。
for f in $RC_FILES; do
  snap="$SNAP_DIR/$(basename "$f")"
  if [ -f "$snap" ]; then
    if [ ! -f "$f" ]; then
      n9=0; note "$f 被删除(PRE 存在)"
    else
      if grep -qxF "$PATH_LINE" "$f" 2>/dev/null || grep -qxF "$FNM_HOOK_LINE" "$f" 2>/dev/null; then
        n9=0; note "$f 仍有 setup-coder 注入行残留"
      fi
      # 规范化比较:去尾部空白行后内容应一致(允许行内 trim 差异,与回滚的 trim 语义对齐)
      _now=$(cat "$f"); _pre=$(cat "$snap")
      if [ "$_now" != "$_pre" ]; then
        # 行级宽松比较:忽略每行首尾空白与空行(回滚重写可能裁尾部空行)
        _norm_now=$(printf '%s\n' "$_now" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' | grep -v '^$' || true)
        _norm_pre=$(printf '%s\n' "$_pre" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' | grep -v '^$' || true)
        if [ "$_norm_now" != "$_norm_pre" ]; then
          n9=0; note "$f 内容与 PRE 快照有实质差异(见 DIFF)"
          diff "$snap" "$f" | head -8 | sed 's/^/  DIFF /'
        fi
      fi
    fi
  else
    # PRE 不存在:现在要么不存在,要么内容为空
    if [ -f "$f" ] && [ -n "$(tr -d '[:space:]' < "$f" 2>/dev/null)" ]; then
      n9=0; note "$f 在 PRE 不存在,现有非空内容(残留)"
    fi
  fi
done
# 9b: 前缀不存在
[ -d "$PREFIX" ] && { n9=0; note "前缀仍存在"; }
# 9c: npmrc 状态
if [ "$PRE_HAD_NPMRC" = "1" ]; then
  cmp -s "$SNAP_DIR/user-npmrc" "$HOME/.npmrc" || { n9=0; note "~/.npmrc 与快照不一致"; }
else
  [ -e "$HOME/.npmrc" ] && { n9=0; note "~/.npmrc 被新建"; }
fi
# 9d: fnm 目录状态
if [ "$PRE_HAD_FNM_DIR" = "0" ]; then
  [ -d "$FNM_DIR" ] && { n9=0; note "fnm 目录残留:$FNM_DIR"; }
else
  # 原有 fnm:不得多出 setup-coder 装的版本
  if [ -d "$FNM_NODE_BASE" ]; then
    for d in "$FNM_NODE_BASE"/v*; do
      [ -d "$d" ] || continue
      b=$(basename "$d")
      case " $PRE_FNM_VERSIONS " in *" $b "*) : ;; *) n9=0; note "fnm 新增版本目录未清理:$b" ;; esac
    done
  fi
fi
if [ $n9 = 1 ]; then
  ok 9 "机器与 PRE 快照一致,无残留 $restored_fnm_note"
else
  bad 9 "存在残留(见 NOTE;EXIT trap 的 cleanup 已尽力恢复)"
fi

# --------------------------------- 汇总 ---------------------------------------
say "------------------------------------------------------------------"
for r in "${RESULTS[@]}"; do say "$r"; done
say "ACCEPT-SUMMARY pass=$PASS fail=$FAIL"
rm -rf "$WORK_DIR"
if [ $FAIL -gt 0 ]; then exit 1; fi
exit 0
