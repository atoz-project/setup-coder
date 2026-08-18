# install.ps1 —— setup-coder 的 One-liner 自举脚本(Windows)
#
# 用法(One-liner,见 CONTEXT.md):
#   irm <本脚本地址> | iex
#
# 兼容 Windows PowerShell 5.1 与 PowerShell 7+(pwsh);
# 经 irm | iex 管道执行成立:不依赖脚本文件路径,也不用 exit(避免关掉用户窗口),出错一律 throw。
#
# 职责(脚本只做"下载二进制并转交",逻辑全在二进制里):
#   探测平台/架构 → 取 latest.json → 按容错链下载对应二进制
#   → sha256 校验 → 放入 %USERPROFILE%\.setup-coder\bin\ → 执行 setup-coder install
#   → 兜底:把 bin 目录幂等写入用户 PATH(HKCU Environment + 广播 WM_SETTINGCHANGE)
# 例外(ARCHITECTURE.md):PATH 持久化兜底是工单 #9 的实战修复——二进制写注册表
# 在用户现场可能失败(见 GitHub issue #9),One-liner 作为用户入口必须在成功后
# 亲自确认 PATH 已持久化,否则小白「装完命令找不到」无法自救。

# ===== 下载容错链配置(维护者只改这里)=====
#
# 容错链顺序:OSS → Gitee → GitHub(加速前缀依次试,最后直连)。
# OSS / Gitee 镜像根 URL 留空 = 跳过该源;填上域名即生效(结尾有无 / 均可,脚本自动归一)。
# 镜像内容 = GitHub Release 根目录整目录拷贝(平铺),因此:
#   镜像上的 latest.json = <镜像根> + /latest.json
#   镜像上的二进制       = <镜像根> + /<latest.json 里的 path 字段>
$OSS_ROOT = ""
$GITEE_ROOT = ""
# GitHub 加速前缀(ghproxy 类),按顺序尝试;全部失败后自动直连 GitHub。
# 这类公共服务时效性强,失效时换一个即可,格式:<前缀> + 完整 GitHub URL。
$GITHUB_PREFIXES = @("https://ghfast.top/", "https://gh-proxy.com/", "https://ghproxy.net/")

$GITHUB_REPO = "atoz-project/setup-coder"
# ===================================

$ErrorActionPreference = 'Stop'
# PS5.1 的 Invoke-WebRequest 进度条会严重拖慢下载,关掉
$ProgressPreference = 'SilentlyContinue'
# 老系统(Win7/早期 Win10)默认 TLS 版本过低,GitHub 会拒绝握手;强制 TLS 1.2
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }

function Say([string]$msg)  { Write-Host $msg }
function Step([string]$msg) { Write-Host "==> $msg" -ForegroundColor Blue }
function Warn([string]$msg) { Write-Host "提醒: $msg" -ForegroundColor Yellow }
function Fail([string]$msg) { throw "失败: $msg" }

# 去掉镜像根 URL 尾部 /,空串归一为 $null(= 跳过该源)
function Normalize-Root([string]$root) {
  if ([string]::IsNullOrEmpty($root)) { return $null }
  return $root.TrimEnd('/')
}

# latest.json 的候选 URL 列表,按容错链顺序
function Get-LatestJsonUrls {
  $urls = @()
  $oss   = Normalize-Root $OSS_ROOT
  $gitee = Normalize-Root $GITEE_ROOT
  if ($oss)   { $urls += "$oss/latest.json" }
  if ($gitee) { $urls += "$gitee/latest.json" }
  $ghLatest = "https://github.com/$GITHUB_REPO/releases/latest/download/latest.json"
  foreach ($p in $GITHUB_PREFIXES) { $urls += "$p$ghLatest" }
  $urls += $ghLatest
  return $urls
}

# 二进制的候选 URL 列表,按容错链顺序;$path 为 latest.json 的 path 字段,$url 为 url 字段
function Get-BinaryUrls([string]$path, [string]$url) {
  $urls = @()
  $oss   = Normalize-Root $OSS_ROOT
  $gitee = Normalize-Root $GITEE_ROOT
  if ($oss)   { $urls += "$oss/$path" }
  if ($gitee) { $urls += "$gitee/$path" }
  foreach ($p in $GITHUB_PREFIXES) { $urls += "$p$url" }
  $urls += $url
  return $urls
}

# 把 $dir 幂等追加进用户 PATH(HKCU\Environment\Path)。
# 返回 $true = 本次有写入;$false = 已存在,无副作用(重跑/幂等轮安全)。
# 读原始值(DoNotExpandEnvironmentNames)并保留原有类型(用户 PATH 常为 REG_EXPAND_SZ,
# 若用 [Environment]::SetEnvironmentVariable 会把它降级成 REG_SZ,破坏 %VAR% 展开)。
# 幂等比较规则与二进制内 windows_path_merge 一致:忽略大小写、首尾空白与尾部 \ /。
function Ensure-UserPath([string]$dir) {
  $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
  try {
    $raw = $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $existing = if ($null -eq $raw) { '' } else { [string]$raw }
    $target = $dir.Trim().TrimEnd('\', '/')
    foreach ($entry in ($existing -split ';')) {
      $norm = $entry.Trim().TrimEnd('\', '/')
      if ($norm -and ($norm -ieq $target)) { return $false }
    }
    $merged = if ($existing.Trim().Length -eq 0) { $dir.Trim() } else { $existing.TrimEnd(';') + ';' + $dir.Trim() }
    $kind = if ($null -ne $raw) { $key.GetValueKind('Path') } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
    $key.SetValue('Path', $merged, $kind)
    return $true
  } finally {
    $key.Dispose()
  }
}

# 广播 WM_SETTINGCHANGE,通知资源管理器等外壳程序环境变量已变(新开的终端才能读到新 PATH)。
# 失败不致命:即便广播没发出去,注销重登或重启后同样生效,只 Warn 不 throw。
function Broadcast-EnvironmentChange {
  try {
    if (-not ('SetupCoder.NativeBroadcast' -as [type])) {
      $sig = '[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)] public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);'
      Add-Type -MemberDefinition $sig -Name 'NativeBroadcast' -Namespace 'SetupCoder' -ErrorAction Stop
    }
    $HWND_BROADCAST = [System.IntPtr]0xffff
    $WM_SETTINGCHANGE = 0x1Au
    $SMTO_ABORTIFHUNG = 0x2u
    $result = [System.UIntPtr]::Zero
    [void][SetupCoder.NativeBroadcast]::SendMessageTimeout($HWND_BROADCAST, $WM_SETTINGCHANGE, [System.UIntPtr]::Zero, 'Environment', $SMTO_ABORTIFHUNG, 5000, [ref]$result)
  } catch {
    Warn "环境变量变更广播未发出(不影响写入结果;注销重登或重启后同样生效)。"
  }
}

# 下载 $url 到 $outFile,成功返回 $true,失败(超时/404/断连)返回 $false 由调用方换源
function Fetch([string]$url, [string]$outFile) {
  try {
    Invoke-WebRequest -Uri $url -OutFile $outFile -TimeoutSec 120 -UseBasicParsing
    return ((Test-Path $outFile) -and ((Get-Item $outFile).Length -gt 0))
  } catch {
    return $false
  }
}

try {
  Say "setup-coder 一键安装"
  Say "===================="

  Step "第 1 步:识别你的电脑平台和 CPU 架构"
  # 32 位 PowerShell 跑在 64 位系统上时 PROCESSOR_ARCHITECTURE=x86,需看 ARCHITEW6432
  $arch = $env:PROCESSOR_ARCHITECTURE
  $archWow = $env:PROCESSOR_ARCHITEW6432
  if ($arch -ne 'AMD64' -and $archWow -ne 'AMD64') {
    Fail "暂不支持 $arch 架构的 Windows。目前仅支持 64 位(x64)Windows。"
  }
  $target = 'win-x64'
  Say "识别结果:$target(Windows 64 位)"

  Step "第 2 步:获取最新版本信息(latest.json)"
  Say "会依次尝试多个下载源,某个源连不上会自动换下一个,请稍等。"
  $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("setup-coder-" + [System.Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $tmp -Force | Out-Null
  $jsonFile = Join-Path $tmp 'latest.json'
  $jsonOk = $false
  foreach ($u in (Get-LatestJsonUrls)) {
    Say "尝试:$u"
    if (Fetch $u $jsonFile) { Say "获取成功。"; $jsonOk = $true; break }
    Warn "这个源连不上,换下一个……"
  }
  if (-not $jsonOk) {
    Fail "所有下载源都连不上 latest.json。`n可能原因:当前网络完全无法访问 GitHub 及镜像。`n建议:检查网络后重试;若反复失败,请到 https://github.com/$GITHUB_REPO/issues 反馈。"
  }

  $latest = Get-Content $jsonFile -Raw -Encoding UTF8 | ConvertFrom-Json
  $plat = $latest.platforms.$target
  if (-not $plat -or -not $plat.path -or -not $plat.url -or -not $plat.sha256) {
    Fail "latest.json 里找不到 $target 的下载信息,可能是版本索引损坏,请向项目反馈。"
  }

  Step "第 3 步:下载 setup-coder 二进制($($plat.path))"
  $binTmp = Join-Path $tmp $plat.path
  $binOk = $false
  foreach ($u in (Get-BinaryUrls $plat.path $plat.url)) {
    Say "尝试:$u"
    if (Fetch $u $binTmp) {
      Say "下载完成,正在校验文件完整性(sha256)……"
      $hash = (Get-FileHash -Path $binTmp -Algorithm SHA256).Hash.ToLower()
      if ($hash -eq $plat.sha256.ToLower()) { Say "校验通过。"; $binOk = $true; break }
      Warn "这个源下载的文件校验不一致(可能传输损坏),换下一个源重试……"
    } else {
      Warn "这个源连不上,换下一个……"
    }
  }
  if (-not $binOk) {
    Fail "所有下载源都拿不到完好的二进制。`n可能原因:网络不稳定导致文件反复损坏,或镜像内容过期。`n建议:稍后重试;若反复失败,请到 https://github.com/$GITHUB_REPO/issues 反馈。"
  }

  Step "第 4 步:安装到 %USERPROFILE%\.setup-coder\bin\"
  $binDir = Join-Path $env:USERPROFILE '.setup-coder\bin'
  New-Item -ItemType Directory -Path $binDir -Force | Out-Null
  $dest = Join-Path $binDir 'setup-coder.exe'
  Copy-Item $binTmp $dest -Force
  Say "已放好:$dest"

  Step "第 5 步:启动安装(setup-coder install)"
  Say "接下来由 setup-coder 自动装好前置依赖(Node.js、git)和各 Tool,全程无需输入。"
  & $dest install
  if ($LASTEXITCODE -ne 0) {
    Fail "二进制已就位,但 setup-coder install 执行失败。`n你可以稍后手动重跑:`"$dest`" install`n若反复失败,请到 https://github.com/$GITHUB_REPO/issues 反馈。"
  }

  Step "第 6 步:把 bin 目录写入用户 PATH(持久化)"
  # 兜底写入(工单 #9):二进制 install 内部也会写,这里作为用户入口亲自确认,
  # 保证「装完新开终端命令可用」。幂等:已存在则不重复写、不重复广播。
  if (Ensure-UserPath $binDir) {
    Say "已写入用户环境变量 Path:$binDir"
    Broadcast-EnvironmentChange
  } else {
    Say "用户环境变量 Path 已包含 $binDir,无需改动。"
  }
  # 当前窗口也立即可用(持久化对新开终端生效,本窗口手动补一份)
  if (($env:Path -split ';') -notcontains $binDir) { $env:Path = "$binDir;$env:Path" }

  Say ""
  Say "全部完成!PATH 已持久化,当前窗口可直接使用;重新打开终端窗口同样生效。"
} catch {
  Write-Host $_ -ForegroundColor Red
  Write-Host "安装未完成。你可以把上面的错误信息截图,到 https://github.com/$GITHUB_REPO/issues 反馈。" -ForegroundColor Red
} finally {
  # 临时目录无论成败都清理(irm|iex 场景下也不能留垃圾)
  if ($tmp -and (Test-Path $tmp)) { Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue }
}
