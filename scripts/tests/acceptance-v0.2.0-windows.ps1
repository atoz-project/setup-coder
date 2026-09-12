#requires -Version 5.1
<#
=============================================================================
 accept-v0.2.0-windows.ps1 — 真机验收脚本(Windows Server 2025,PowerShell)
 setup-coder v0.2.0「Node 前置复用,不劫持环境」特性

 运行方式:ECS RunCommand(RunPowerShellScript)以【目标普通用户】身份执行,
   绝不用 Administrator/SYSTEM(HKCU、%USERPROFILE%、%LOCALAPPDATA% 必须属于
   被验收的真实用户)。Windows PowerShell 5.1 与 pwsh 7+ 均可(RunCommand 默认 5.1)。

 流程(幂等、自清理):
   PRE 快照(HKCU Path 原值+类型、两个 PowerShell profile、~/.npmrc、前缀、
   fnm 目录)→ 下载 v0.2.0 二进制 → install → 断言(1-7)
   → uninstall --yes → 断言回滚(8-9)
  → 若 setup-coder 代装了 fnm,按 uninstall 打印的「手工移除步骤」删除
    %APPDATA%\fnm(fnm 真实默认数据目录,Roaming),恢复到 PRE 状态。
   任何 FAIL 都触发同样的清理(finally),退出码非 0。

 输出协议:每检查点一行 `ACCEPT <n> PASS|FAIL: <描述>`;结尾汇总,
   任一 FAIL → exit 1(RunCommand 捕获 ExitCode)。

 注意:Windows 侧实现尚未真机验证(发布注记)——本脚本对 HKCU PATH 机制、
 profile 钩子位置、.cmd shim 形态、fnm 布局一律按代码契约【严格断言】,
 任何偏差都是真实的验收发现。
=============================================================================
#>
$ErrorActionPreference = 'Continue'   # 不因单条错误中止;检查结果显式记录
$ProgressPreference = 'SilentlyContinue'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }
[Console]::OutputEncoding = [Text.Encoding]::UTF8   # 让中文断言输出可读

# -------------------- 常量(与 v0.2.0 代码逐字对齐,src/platform/windows.rs) ----
$Ver       = 'v0.2.0'
$Asset     = 'setup-coder-win-x64.exe'
$DlUrl     = "https://github.com/atoz-project/setup-coder/releases/download/$Ver/$Asset"
$GhPrefixes = @('https://ghfast.top/', 'https://gh-proxy.com/', 'https://ghproxy.net/', '')

$Prefix    = Join-Path $env:USERPROFILE '.setup-coder'
$BinDir    = Join-Path $Prefix 'bin'
$StatePath = Join-Path $Prefix 'state.json'
$FloorAll  = [version]'22.19.0'      # 全量工具(codex/claude/pi)Node 下限
# fnm 真实默认数据目录:%APPDATA%\fnm(Roaming——fnm v1.39.0 经 etcetera
# `data_dir()` 取 Roaming,实机证据 F1:node 装进 Roaming\fnm\node-versions,
# v0.2.0 却按 %LOCALAPPDATA%\fnm 解析必失败);FNM_DIR 环境变量优先
$FnmDir    = if ($env:FNM_DIR) { $env:FNM_DIR } else { Join-Path $env:APPDATA 'fnm' }   # fnm_default_dir()
$FnmNodeBase = Join-Path $FnmDir 'node-versions'
# inject_fnm_hook() 写入的两个 profile(代码硬编码,非运行时 $PROFILE)
$ProfilePaths = @(
  (Join-Path $env:USERPROFILE 'Documents\PowerShell\Microsoft.PowerShell_profile.ps1'),
  (Join-Path $env:USERPROFILE 'Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1')
)
# fnm_hook_line_powershell() 逐字:
$FnmHookLine = '& "' + (Join-Path $FnmDir 'fnm.exe') + '" env --use-on-cd | Out-String | Invoke-Expression  # setup-coder fnm'   # fnm_hook_line_powershell(fnm_exe),绝对路径不依赖 PATH
$NvmDirDefault = Join-Path $env:APPDATA 'nvm'               # nvm-windows 默认(NVM_DIR 优先)

$script:Pass = 0; $script:Fail = 0
$Results = New-Object System.Collections.Generic.List[string]
function Say([string]$m)  { Write-Host $m }
function Note([string]$m) { Say "NOTE $m" }
function Ok([int]$n, [string]$what) { $script:Pass++; $line = "ACCEPT $n PASS: $what"; $Results.Add($line); Say $line }
function Bad([int]$n, [string]$what) { $script:Fail++; $line = "ACCEPT $n FAIL: $what"; $Results.Add($line); Say $line }

function Test-VersionGe([version]$a, [version]$b) { return $a -ge $b }

# 读 HKCU\Environment\Path 的【原值】(不展开 %VAR%),与二进制 winreg 行为一致
function Get-UserPathRaw {
  $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')
  if ($null -eq $k) { return $null }
  try { return $k.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
  finally { $k.Close() }
}
function Get-UserPathKind {
  $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')
  if ($null -eq $k) { return 'KeyMissing' }
  try {
    try { return $k.GetValueKind('Path').ToString() } catch { return 'ValueMissing' }
  } finally { $k.Close() }
}
# PATH 条目匹配:与 windows_path_contains 同一归一化(去尾斜杠、忽略大小写、trim)
function Test-UserPathContains([string]$raw, [string]$dir) {
  if ($null -eq $raw) { return $false }
  $norm = { param($s) if ($null -eq $s) { $s = '' }; $s.Trim().TrimEnd('\', '/').ToLowerInvariant() }
  $target = & $norm $dir
  foreach ($e in ($raw -split ';')) { if ((& $norm $e) -eq $target) { return $true } }
  return $false
}
# F5:PS 5.1 把传入 [string] 形参的 $null 强转为 ''——`$null -eq $value` 永假,
# 快照为空时会把 Path 写成【空 ExpandString】而不是删除(F4 同类隐患)。
# 故空值/空串一律按「快照时不存在该值」处理:删除。
function Set-UserPathRaw([string]$value, [string]$kindName) {
  $k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
  try {
    if ([string]::IsNullOrEmpty($value)) { $k.DeleteValue('Path', $false); return }
    $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
    if ($kindName -eq 'String') { $kind = [Microsoft.Win32.RegistryValueKind]::String }
    $k.SetValue('Path', $value, $kind)
  } finally { $k.Close() }
}

# ------------------------------- 幂等清理器 -----------------------------------
$SnapDir = $null
# 全部键在声明处初始化(F5):cleanup 可能在快照段任一时刻被触发,空快照也
# 必须安全——ProfileExisted 永远非 $null,Snapshotted 标记快照是否完成
$Pre = @{ ProfileExisted = @{}; UserPathRaw = $null; UserPathKind = 'ValueMissing'; HadNpmrc = $false; HadFnmDir = $false; FnmVersions = @(); Snapshotted = $false }
function Invoke-Cleanup {
  # 1) setup-coder 自身卸载(清单驱动精确回滚)
  try {
    $self = Join-Path $BinDir 'setup-coder.exe'
    if (Test-Path $self) {
      $p = Start-Process -FilePath $self -ArgumentList 'uninstall','--yes' -Wait -PassThru `
           -NoNewWindow -RedirectStandardOutput "$env:TEMP\acc-un-cleanup.out" -RedirectStandardError "$env:TEMP\acc-un-cleanup.err"
    }
    if (Test-Path $Prefix) {
      # Windows 自删残留:前缀可能只剩 bin\setup-coder.exe(正在运行的 exe 删不掉自己属设计)
      Remove-Item -Recurse -Force $Prefix -ErrorAction SilentlyContinue
    }
  } catch { Note "cleanup(uninstall) error: $($_.Exception.Message)" }
  # 2) 代装 fnm 资产:uninstall 按设计保留 → 验收按其提示手工恢复 PRE 状态
  try {
    if (-not $Pre.HadFnmDir -and (Test-Path $FnmDir)) {
      Remove-Item -Recurse -Force $FnmDir -ErrorAction SilentlyContinue
    } elseif ($Pre.HadFnmDir -and (Test-Path $FnmNodeBase)) {
      Get-ChildItem $FnmNodeBase -Directory -ErrorAction SilentlyContinue | ForEach-Object {
        if ($Pre.FnmVersions -notcontains $_.Name) {
          Remove-Item -Recurse -Force $_.FullName -ErrorAction SilentlyContinue
        }
      }
    }
  } catch { Note "cleanup(fnm) error: $($_.Exception.Message)" }
  # 3) profile 整文件恢复(快照含用户原有内容,无误删风险)
  foreach ($f in $ProfilePaths) {
    $snap = Join-Path $SnapDir ("prof-" + ($f -replace '[\\/:*?"<>|]', '_'))
    try {
      # F5:$Pre.ProfileExisted 在快照前/快照中断时可能为空 hashtable 或缺键,
      # 索引 $null 会抛「无法对 Null 数组进行索引」;缺键一律按「快照时不存在」
      $existed = $false
      if ($Pre.ProfileExisted -and $Pre.ProfileExisted.ContainsKey($f)) { $existed = [bool]$Pre.ProfileExisted[$f] }
      if (Test-Path $snap) { Copy-Item -Force $snap $f }
      elseif ($Pre.Snapshotted -and (Test-Path $f) -and -not $existed) {
        # 快照完成后才允许删「快照时不存在」的文件(钩子是 install 新建的);
        # 快照未完成时宁可留着,绝不误删用户原有 profile
        Remove-Item -Force $f -ErrorAction SilentlyContinue
      }
    } catch { Note "cleanup(profile $f) error: $($_.Exception.Message)" }
  }
  # 4) HKCU Path 恢复原值与原始类型(含 REG_EXPAND_SZ;快照前不存在则删除该值)
  try {
    Set-UserPathRaw $Pre.UserPathRaw $Pre.UserPathKind
  } catch { Note "cleanup(registry Path) error: $($_.Exception.Message)" }
  # 5) 用户 ~/.npmrc 恢复
  try {
    $npmrc = Join-Path $env:USERPROFILE '.npmrc'
    if ($Pre.HadNpmrc) { Copy-Item -Force (Join-Path $SnapDir 'user-npmrc') $npmrc }
    elseif (Test-Path $npmrc) { Remove-Item -Force $npmrc -ErrorAction SilentlyContinue }
  } catch { Note "cleanup(npmrc) error: $($_.Exception.Message)" }
  if ($SnapDir -and (Test-Path $SnapDir)) { Remove-Item -Recurse -Force $SnapDir -ErrorAction SilentlyContinue }
  Say 'CLEANUP done (machine restored to pre-run state as far as possible)'
}

# ------------------------------- PRE:环境基线 ---------------------------------
Say '=================================================================='
Say (" setup-coder {0} 验收(Windows)— {1:yyyy-MM-ddTHH:mm:ss} — user={2}" -f $Ver, (Get-Date), $env:USERNAME)
Say '=================================================================='
Note "os=$([Environment]::OSVersion.VersionString) ps=$($PSVersionTable.PSVersion)"

$SnapDir = Join-Path $env:TEMP ("setup-coder-accept-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $SnapDir | Out-Null

# git 前置:Windows 侧缺 git 会装便携 MinGit 进前缀(随前缀删除,不算系统改动)
$gitCmd = Get-Command git.exe -ErrorAction SilentlyContinue
Note ("git on PATH: " + ($(if ($gitCmd) { $gitCmd.Source } else { 'NO — install 将装前缀内 MinGit(随前缀删除)' })))

# --- 探测 Node 三来源(镜像 detect_node_facts,Windows) ---
$preNodePath = $null; $preNodeVer = $null
$nodeCmd = Get-Command node.exe -ErrorAction SilentlyContinue
if ($nodeCmd) {
  $preNodePath = $nodeCmd.Source
  $vout = (& $preNodePath --version 2>$null)
  if ($vout -match 'v?(\d+\.\d+\.\d+)') { $preNodeVer = [version]$Matches[1] }
}
$preHadNvm = $false
if ($env:NVM_DIR -and (Test-Path $env:NVM_DIR)) { $preHadNvm = $true }
elseif (Test-Path $NvmDirDefault) { $preHadNvm = $true }

$preHadFnm = $false
$fnmCmd = Get-Command fnm.exe -ErrorAction SilentlyContinue
if ($fnmCmd -or (Test-Path $FnmDir)) { $preHadFnm = $true }
# 与 detect_fnm_windows 一致:profile 钩子痕迹也算「已装 fnm」
if (-not $preHadFnm) {
  foreach ($f in $ProfilePaths) {
    if ((Test-Path $f) -and (Select-String -Path $f -Pattern 'fnm env' -Quiet -ErrorAction SilentlyContinue)) { $preHadFnm = $true; break }
  }
}
$preFnmVersions = @()
if (Test-Path $FnmNodeBase) { $preFnmVersions = @(Get-ChildItem $FnmNodeBase -Directory | ForEach-Object Name) }

# --- decide() 期望值(与 node_plan.rs 锁定优先级一致) ---
# detect_bare_node_windows 用 where.exe node——nvm-windows 的 symlink 或 fnm 的
# PATH shim 命中同样会走 ReuseBareNode(实现口径如此,期望与之一致)。
$expectSource = ''
if ($preNodeVer -and (Test-VersionGe $preNodeVer $FloorAll)) { $expectSource = 'user_bare' }
elseif ($preHadNvm) { $expectSource = 'user_nvm' }
elseif ($preHadFnm) { $expectSource = 'user_fnm' }
else { $expectSource = 'user_fnm' }   # InstallFnm 落账同为 user_fnm,靠 FnmHook 注入记录区分
$expectFnmHook = $false
if (-not ($preNodeVer -and (Test-VersionGe $preNodeVer $FloorAll)) -and -not $preHadNvm -and -not $preHadFnm) {
  $expectFnmHook = $true
}
Say ("EXPECT node.source={0} (bare={1}@{2} nvm={3} fnm={4}; floor={5})" -f `
    $expectSource, $(if ($preNodeVer) { "v$preNodeVer" } else { 'none' }), $(if ($preNodePath) { $preNodePath } else { 'none' }), `
    $preHadNvm, $preHadFnm, $FloorAll)
Say "EXPECT fnm_hook_injected=$([int]$expectFnmHook) (1 = setup-coder 代装 fnm 分支)"

# --- 快照 ---
$Pre.UserPathRaw  = Get-UserPathRaw
$Pre.UserPathKind = Get-UserPathKind
Note ("HKCU Path kind=" + $Pre.UserPathKind + " len=" + $(if ($Pre.UserPathRaw) { $Pre.UserPathRaw.Length } else { 0 }))
$Pre.ProfileExisted = @{}
foreach ($f in $ProfilePaths) {
  $Pre.ProfileExisted[$f] = Test-Path $f
  if ($Pre.ProfileExisted[$f]) {
    Copy-Item -Force $f (Join-Path $SnapDir ("prof-" + ($f -replace '[\\/:*?"<>|]', '_')))
  }
}
$Pre.HadNpmrc = Test-Path (Join-Path $env:USERPROFILE '.npmrc')
if ($Pre.HadNpmrc) { Copy-Item -Force (Join-Path $env:USERPROFILE '.npmrc') (Join-Path $SnapDir 'user-npmrc') }
$Pre.HadFnmDir = Test-Path $FnmDir
$Pre.FnmVersions = $preFnmVersions
$Pre.Snapshotted = $true   # 快照完成标记:cleanup 的「删除快照时不存在的文件」以此为闸
if (Test-Path $Prefix) {
  Say "FATAL: $Prefix 已存在——本机已有 setup-coder 安装,验收会干扰它。请先手工卸载再跑。"
  Invoke-Cleanup; exit 2
}
# ------------------------------- 下载二进制 -----------------------------------
$WorkDir = Join-Path $env:TEMP ("setup-coder-bin-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $WorkDir | Out-Null
$Bin = Join-Path $WorkDir 'setup-coder.exe'
$dlOk = $false
# 预置二进制通道:SETUP_CODER_ACCEPT_BIN 指向本地 exe 时跳过下载(验收
# 未发布修复版/离线重跑用);为空则按容错链下载 $Ver 正式资产
if ($env:SETUP_CODER_ACCEPT_BIN -and (Test-Path $env:SETUP_CODER_ACCEPT_BIN)) {
  Copy-Item $env:SETUP_CODER_ACCEPT_BIN $Bin -Force
  Note ("using staged binary: {0} ({1} bytes)" -f $env:SETUP_CODER_ACCEPT_BIN, (Get-Item $Bin).Length)
  $dlOk = $true
} else {
  foreach ($pre in $GhPrefixes) {
    $url = "$pre$DlUrl"
    Note "trying download: $url"
    try {
      Invoke-WebRequest -Uri $url -OutFile $Bin -TimeoutSec 300 -UseBasicParsing -ErrorAction Stop
      if ((Get-Item $Bin).Length -gt 1MB) { $dlOk = $true; break }
    } catch { Note "download failed: $($_.Exception.Message)" }
  }
}
if (-not $dlOk) {
  Bad 1 "下载 $Asset 失败(GitHub 直连与加速前缀均不可达)"
  Say "ACCEPT-SUMMARY pass=$($script:Pass) fail=$($script:Fail) (aborted at download)"
  Invoke-Cleanup; exit 1
}
Note ("downloaded {0} bytes" -f (Get-Item $Bin).Length)

# 子进程跑 setup-coder(捕获输出到文件,便于中文行断言)
function Invoke-SetupCoder([string[]]$cmdArgs, [string]$logBase) {
  $out = "$logBase.out"; $err = "$logBase.err"
  $p = Start-Process -FilePath $Bin -ArgumentList $cmdArgs -Wait -PassThru -NoNewWindow `
       -RedirectStandardOutput $out -RedirectStandardError $err
  return $p.ExitCode
}
function Get-LogText([string]$logBase) {
  $t = ''
  if (Test-Path "$logBase.out") { $t += [IO.File]::ReadAllText("$logBase.out", [Text.Encoding]::UTF8) }
  if (Test-Path "$logBase.err") { $t += [IO.File]::ReadAllText("$logBase.err", [Text.Encoding]::UTF8) }
  return $t
}

$aborted = $false
try {

# -------------------- ACCEPT 1:install 端到端 ---------------------------------
$installLog = Join-Path $SnapDir 'install'
$rc = Invoke-SetupCoder @('install') $installLog
if ($rc -eq 0) {
  Ok 1 'install 端到端成功(exit 0;工具经前缀内 npm 安装并冒烟)'
} else {
  Bad 1 "install 失败(exit $rc;见下方日志尾)"
  (Get-LogText $installLog) -split "`n" | Select-Object -Last 25 | ForEach-Object { Say "  INSTALL-LOG $_" }
}
foreach ($t in 'codex','claude','pi') {
  if (-not (Test-Path (Join-Path $BinDir "$t.cmd"))) { Bad 1 "缺少 shim:$BinDir\$t.cmd" }
}

# -------------------- ACCEPT 2:Node 来源与探测前状态一致 ----------------------
$state = $null
if (Test-Path $StatePath) {
  try { $state = [IO.File]::ReadAllText($StatePath, [Text.Encoding]::UTF8) | ConvertFrom-Json } catch { }
}
if ($null -ne $state -and $state.node.source -eq $expectSource) {
  Ok 2 "Node 来源与决策一致:state.json source=$($state.node.source)(期望 $expectSource)"
} else {
  Bad 2 "Node 来源不符:state.json source=$(if ($state) { $state.node.source } else { 'missing' })(期望 $expectSource)"
}

# -------------------- ACCEPT 3:去劫持证明(最小 PATH 跑 .cmd shim) -------------
# 用 cmd.exe /c 以【仅系统目录】的 PATH 拉起 shim:不含任何 node/nvm/fnm 目录。
# 进程级 PATH 覆盖不触碰注册表,验证结束即恢复本进程原 PATH。
$dehijackOk = $true
$savedPath = $env:PATH
try {
  $minimal = "$env:SystemRoot\System32;$env:SystemRoot"
  foreach ($t in 'codex','claude','pi') {
    $shim = Join-Path $BinDir "$t.cmd"
    if (-not (Test-Path $shim)) { $dehijackOk = $false; continue }
    # cmd.exe /c 读的是进程环境 → 先改本进程 PATH,起完立即恢复
    $env:PATH = $minimal
    $p = Start-Process -FilePath "$env:SystemRoot\System32\cmd.exe" `
         -ArgumentList '/c', "`"$shim`" --version" -Wait -PassThru -NoNewWindow `
         -RedirectStandardOutput "$SnapDir\dh-$t.out" -RedirectStandardError "$SnapDir\dh-$t.err"
    $env:PATH = $savedPath
    $outTxt = ''
    if (Test-Path "$SnapDir\dh-$t.out") { $outTxt = [IO.File]::ReadAllText("$SnapDir\dh-$t.out", [Text.Encoding]::UTF8).Trim() }
    if ($p.ExitCode -ne 0 -or [string]::IsNullOrWhiteSpace($outTxt)) {
      $dehijackOk = $false
      Note "de-hijack run failed: $t exit=$($p.ExitCode) out='$outTxt'"
    }
  }
} finally { $env:PATH = $savedPath }
if ($dehijackOk) {
  Ok 3 '去劫持:仅 System32 的 PATH 下 codex/claude/pi .cmd shim --version 均 exit 0 且有输出'
} else {
  Bad 3 '去劫持:最小 PATH 下有 shim 不能正常启动(见 NOTE)'
}

# -------------------- ACCEPT 4:前缀无 node/npm;HKCU PATH 仅注入 bin/ ----------
$n4 = $true
foreach ($x in 'node.exe','node','npm.cmd','npm','npx.cmd','npx') {
  if (Test-Path (Join-Path $BinDir $x)) { $n4 = $false; Note "prefix bin 含违禁文件:$x" }
}
# 前缀内(除 cache\ 下载缓存外)不允许出现 node 发行布局。只匹配【文件】——
# npm 包内容里可能有名为 node 的目录(实机:pi 的依赖 @earendil-works/chord
# 带 dist\node\ 目录),目录不是 Node 运行时,匹配目录会误报;语义与 linux
# 脚本 `find -type f -name node` 对齐
$nodeInPrefix = Get-ChildItem $Prefix -Recurse -Force -File -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -notlike "$Prefix\cache\*" -and $_.Name -in 'node.exe','node' } |
  Select-Object -First 1
if ($nodeInPrefix) { $n4 = $false; Note "prefix 内(除 cache)发现 node:$($nodeInPrefix.FullName)" }
# HKCU Path 原值里应恰好新增 bin/ 一条;且不含任何 node 目录的注入
$pathNow = Get-UserPathRaw
$binCount = 0
if ($pathNow) { foreach ($e in ($pathNow -split ';')) { if ($e.Trim().TrimEnd('\','/').ToLowerInvariant() -eq $BinDir.TrimEnd('\','/').ToLowerInvariant()) { $binCount++ } } }
if ($binCount -ne 1) { $n4 = $false; Note "HKCU Path 中 bin/ 出现 $binCount 次(应为恰好 1)" }
$preEntries = @()
if ($Pre.UserPathRaw) { $preEntries = @($Pre.UserPathRaw -split ';' | ForEach-Object { $_.Trim().TrimEnd('\','/').ToLowerInvariant() }) }
$pathEntries = @()
if ($pathNow) { $pathEntries = @($pathNow -split ';') }
foreach ($e in $pathEntries) {
  $ne = $e.Trim().TrimEnd('\','/')
  if ($ne -eq '') { continue }
  if ($preEntries -contains $ne.ToLowerInvariant()) { continue }
  # 新增条目:只能是 bin/
  if ($ne.ToLowerInvariant() -ne $BinDir.TrimEnd('\','/').ToLowerInvariant()) {
    $n4 = $false; Note "HKCU Path 出现非 bin/ 新增条目:$ne"
  }
}
# REG_EXPAND_SZ 类型保留(若 PRE 是 ExpandString,现在必须仍是)
if ($Pre.UserPathKind -eq 'ExpandString' -and (Get-UserPathKind) -ne 'ExpandString') {
  $n4 = $false; Note 'HKCU Path 类型被降级(REG_EXPAND_SZ 未保留)'
}
if ($n4) {
  Ok 4 '前缀内无 node/npm;HKCU Path 唯一新增条目为 bin/ 且 REG_EXPAND_SZ 类型保留'
} else {
  Bad 4 '前缀发现 node/npm,或 HKCU Path 注入异常(见 NOTE)'
}

# -------------------- ACCEPT 5:state.json v2 三值来源 -------------------------
if ($null -ne $state -and $state.version -eq 2 -and $state.node.source -in @('user_bare','user_nvm','user_fnm')) {
  Ok 5 "state.json v2:version=2 且 node.source=$($state.node.source)(三值枚举,无 prefix 值)"
} else {
  Bad 5 "state.json 缺失、version≠2 或 source 非三值枚举(got version=$($state.version) source=$($state.node.source))"
}

# -------------------- ACCEPT 6:用户 ~/.npmrc 未被触碰 --------------------------
$n6 = $true
$userNpmrc = Join-Path $env:USERPROFILE '.npmrc'
if ($Pre.HadNpmrc) {
  $a = [IO.File]::ReadAllBytes((Join-Path $SnapDir 'user-npmrc'))
  $b = if (Test-Path $userNpmrc) { [IO.File]::ReadAllBytes($userNpmrc) } else { @() }
  if (-not [Linq.Enumerable]::SequenceEqual([byte[]]$a, [byte[]]$b)) { $n6 = $false; Note '用户 ~/.npmrc 内容被改动' }
} elseif (Test-Path $userNpmrc) {
  $n6 = $false; Note 'install 新建了用户 ~/.npmrc(不应存在)'
}
$prefixNpmrc = Join-Path $Prefix '.npmrc'
if (-not ((Test-Path $prefixNpmrc) -and (Select-String -Path $prefixNpmrc -Pattern 'registry=https://registry.npmmirror.com' -Quiet))) {
  $n6 = $false; Note '前缀内 .npmrc 缺失或未指向 npmmirror'
}
if ($n6) {
  Ok 6 '用户 ~/.npmrc 未改动;registry 配置只写在前缀内 .npmrc'
} else {
  Bad 6 'npm 配置越界或前缀 .npmrc 异常(见 NOTE)'
}

# -------------------- ACCEPT 7:幂等重跑 ---------------------------------------
$hookCountBefore = 0
foreach ($f in $ProfilePaths) {
  if (Test-Path $f) { $hookCountBefore += ([IO.File]::ReadAllText($f, [Text.Encoding]::UTF8) -split "`n" | Where-Object { $_.Trim() -eq $FnmHookLine }).Count }
}
$pathCountBefore = 0
if ($pathNow) { foreach ($e in ($pathNow -split ';')) { if ($e.Trim().TrimEnd('\','/').ToLowerInvariant() -eq $BinDir.TrimEnd('\','/').ToLowerInvariant()) { $pathCountBefore++ } } }
$fnmExeMtime = $null
$fnmExePath = Join-Path $FnmDir 'fnm.exe'
if (Test-Path $fnmExePath) { $fnmExeMtime = (Get-Item $fnmExePath).LastWriteTimeUtc }

$rerunLog = Join-Path $SnapDir 'rerun'
$rc2 = Invoke-SetupCoder @('install') $rerunLog
$n7 = $true
if ($rc2 -ne 0) { $n7 = $false; Note "重跑 install 失败(exit $rc2)" }
# profile 钩子行数不得增加
$hookCountAfter = 0
foreach ($f in $ProfilePaths) {
  if (Test-Path $f) { $hookCountAfter += ([IO.File]::ReadAllText($f, [Text.Encoding]::UTF8) -split "`n" | Where-Object { $_.Trim() -eq $FnmHookLine }).Count }
}
if ($hookCountAfter -gt $hookCountBefore) { $n7 = $false; Note "profile fnm 钩子行 $hookCountBefore→$hookCountAfter(重复注入)" }
# HKCU Path 中 bin/ 仍应只有一条(无重复追加)
$pathAfter = Get-UserPathRaw
$pathCountAfter = 0
if ($pathAfter) { foreach ($e in ($pathAfter -split ';')) { if ($e.Trim().TrimEnd('\','/').ToLowerInvariant() -eq $BinDir.TrimEnd('\','/').ToLowerInvariant()) { $pathCountAfter++ } } }
if ($pathCountAfter -gt 1) { $n7 = $false; Note "HKCU Path 中 bin/ 出现 $pathCountAfter 次(重复追加)" }
# 代装 fnm 分支:重跑不得重下 fnm(幂等复用 → fnm.exe mtime 不变;弱信号)
if ($expectFnmHook -and $fnmExeMtime -and (Test-Path $fnmExePath)) {
  if ((Get-Item $fnmExePath).LastWriteTimeUtc -ne $fnmExeMtime) { Note 'fnm.exe mtime 变化(可能重装;弱信号,仅记录)' }
}
# source 不得变
$state2 = $null
if (Test-Path $StatePath) { try { $state2 = [IO.File]::ReadAllText($StatePath, [Text.Encoding]::UTF8) | ConvertFrom-Json } catch { } }
if ($null -eq $state2 -or $state2.node.source -ne $expectSource) { $n7 = $false; Note "重跑后 source 变为 $($state2.node.source)" }
if ($n7) {
  Ok 7 '幂等重跑:exit 0,profile 钩子与 HKCU Path 无重复注入,source 不变'
} else {
  Bad 7 '幂等重跑异常(见 NOTE)'
}

# -------------------- ACCEPT 8:uninstall 回滚 ---------------------------------
$unLog = Join-Path $SnapDir 'uninstall'
$selfExe = Join-Path $BinDir 'setup-coder.exe'
$runExe = if (Test-Path $selfExe) { $selfExe } else { $Bin }   # 优先用前缀内本体(与真实用户路径一致)
$p = Start-Process -FilePath $runExe -ArgumentList 'uninstall','--yes' -Wait -PassThru -NoNewWindow `
     -RedirectStandardOutput "$unLog.out" -RedirectStandardError "$unLog.err"
$unText = Get-LogText $unLog
$n8 = $true
if ($p.ExitCode -ne 0) { $n8 = $false; Note "uninstall --yes 退出码 $($p.ExitCode)" }
# 8a: 前缀删除——Windows 自删残留(只剩 bin\setup-coder.exe)属代码内设计(工单 #4)
if (Test-Path $Prefix) {
  $leftovers = Get-ChildItem $Prefix -Recurse -Force -ErrorAction SilentlyContinue
  $onlySelf = $true
  foreach ($it in $leftovers) {
    if ($it.PSIsContainer) { continue }
    if ($it.FullName -ne (Join-Path $BinDir 'setup-coder.exe')) { $onlySelf = $false; break }
  }
  if (-not $onlySelf) { $n8 = $false; Note "uninstall 后前缀残留超出自删豁免:$(($leftovers | Select-Object -First 3).FullName -join ', ')" }
  else { Note '前缀仅剩 setup-coder.exe 自删残留(Windows 设计内;cleanup 将清除)' }
}
# 8b: HKCU Path 中 bin/ 条目回滚
$pathPostUn = Get-UserPathRaw
if (Test-UserPathContains $pathPostUn $BinDir) { $n8 = $false; Note 'HKCU Path 中 bin/ 条目未回滚' }
# 8c: fnm 钩子行回滚(仅代装分支注入过)
if ($expectFnmHook) {
  foreach ($f in $ProfilePaths) {
    if ((Test-Path $f) -and (([IO.File]::ReadAllText($f, [Text.Encoding]::UTF8) -split "`n" | Where-Object { $_.Trim() -eq $FnmHookLine }).Count -gt 0)) {
      $n8 = $false; Note "$f 中 fnm 钩子行未回滚"
    }
  }
}
# 8d: 用户 node/nvm/fnm 保留
if ($preNodePath -and -not (Test-Path $preNodePath)) { $n8 = $false; Note "用户原有 node 消失:$preNodePath" }
if ($preHadNvm -and -not ((Test-Path $NvmDirDefault) -or ($env:NVM_DIR -and (Test-Path $env:NVM_DIR)))) { $n8 = $false; Note '用户 nvm 目录被删' }
if ($Pre.HadFnmDir -and -not (Test-Path $FnmDir)) { $n8 = $false; Note '用户 fnm 目录被删' }
# 8e: 按来源的中文保留提示
$hintOk = $false
switch ($expectSource) {
  'user_bare' { if ($unText -match '你机器上原有的 Node') { $hintOk = $true } }
  'user_nvm'  { if ($unText -match '你的 nvm 与其 Node')   { $hintOk = $true } }
  'user_fnm'  {
    if ($expectFnmHook) { if ($unText -match 'setup-coder 为你安装了 fnm 与 Node') { $hintOk = $true } }
    else { if ($unText -match '你的 fnm 与其 Node') { $hintOk = $true } }
  }
}
if (-not $hintOk) { $n8 = $false; Note '中文保留提示缺失/不匹配(见 uninstall 输出)' }
if ($n8) {
  Ok 8 'uninstall:前缀删除(自删残留豁免),HKCU Path/profile 钩子按记录回滚,用户 Node 资产保留,中文提示正确'
} else {
  Bad 8 'uninstall 回滚异常(见 NOTE)'
}
($unText -split "`n") | Select-Object -Last 8 | ForEach-Object { Say "  UNINSTALL-LOG $_" }

# -------------------- ACCEPT 9:与 PRE 快照零残留 -------------------------------
# 先清理代装 fnm 资产(uninstall 按设计保留;验收负责复原共享机)
$restoredNote = ''
if ($expectFnmHook -and -not $Pre.HadFnmDir -and (Test-Path $FnmDir)) {
  Remove-Item -Recurse -Force $FnmDir -ErrorAction SilentlyContinue
  $restoredNote = "(已按 uninstall 提示手工删除代装 fnm:$FnmDir)"
}
# 清掉 Windows 自删残留
if (Test-Path $Prefix) { Remove-Item -Recurse -Force $Prefix -ErrorAction SilentlyContinue }
$n9 = $true
# 9a: HKCU Path 恢复原值
$pathFinal = Get-UserPathRaw
$pf = ''; $pp = ''
if ($null -ne $pathFinal) { $pf = $pathFinal }
if ($null -ne $Pre.UserPathRaw) { $pp = $Pre.UserPathRaw }
if ($pf -ne $pp) {
  $n9 = $false
  Note "HKCU Path 与 PRE 快照不一致:cleanup 将恢复原值"
  Set-UserPathRaw $Pre.UserPathRaw $Pre.UserPathKind
}
# 9b: profile 与快照一致
foreach ($f in $ProfilePaths) {
  $snap = Join-Path $SnapDir ("prof-" + ($f -replace '[\\/:*?"<>|]', '_'))
  if (Test-Path $snap) {
    $same = $false
    if (Test-Path $f) {
      $same = [Linq.Enumerable]::SequenceEqual([byte[]][IO.File]::ReadAllBytes($snap), [byte[]][IO.File]::ReadAllBytes($f))
    }
    if (-not $same) { $n9 = $false; Note "$f 与 PRE 快照不一致(cleanup 将恢复)" }
  } elseif ((Test-Path $f) -and ((Get-Item $f).Length -gt 0)) {
    $n9 = $false; Note "$f 在 PRE 不存在,现有非空内容(cleanup 将删除)"
  }
}
# 9c: 前缀不存在
if (Test-Path $Prefix) { $n9 = $false; Note '前缀仍存在' }
# 9d: npmrc 状态
if ($Pre.HadNpmrc) {
  $same = (Test-Path $userNpmrc) -and [Linq.Enumerable]::SequenceEqual(
    [byte[]][IO.File]::ReadAllBytes((Join-Path $SnapDir 'user-npmrc')), [byte[]][IO.File]::ReadAllBytes($userNpmrc))
  if (-not $same) { $n9 = $false; Note '~/.npmrc 与快照不一致' }
} elseif (Test-Path $userNpmrc) { $n9 = $false; Note '~/.npmrc 被新建' }
# 9e: fnm 目录状态
if (-not $Pre.HadFnmDir) {
  if (Test-Path $FnmDir) { $n9 = $false; Note "fnm 目录残留:$FnmDir" }
} elseif (Test-Path $FnmNodeBase) {
  Get-ChildItem $FnmNodeBase -Directory -ErrorAction SilentlyContinue | ForEach-Object {
    if ($Pre.FnmVersions -notcontains $_.Name) { $n9 = $false; Note "fnm 新增版本目录未清理:$($_.Name)" }
  }
}
if ($n9) {
  Ok 9 "机器与 PRE 快照一致,无残留 $restoredNote"
} else {
  Bad 9 '存在残留(见 NOTE;cleanup 已尽力恢复)'
}

} finally {
  # --------------------------------- 汇总 -----------------------------------
  Say '------------------------------------------------------------------'
  foreach ($r in $Results) { Say $r }
  Say "ACCEPT-SUMMARY pass=$($script:Pass) fail=$($script:Fail)"
  Invoke-Cleanup
  Remove-Item -Recurse -Force $WorkDir -ErrorAction SilentlyContinue
}
if ($script:Fail -gt 0) { exit 1 }
exit 0
