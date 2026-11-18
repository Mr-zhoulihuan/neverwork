# 构建 NeverWork release 可执行文件并打出 NSIS 安装包。
#
# 用法（从仓库任意位置）：
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1            # 全流程
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1 -SkipBuild # 复用已有 release
#
# 依赖：Rust 工具链、NSIS（winget install NSIS.NSIS）。
# 图标已入库（crates\app\assets\neverwork.ico），编 exe 时由 build.rs 内嵌，安装包也直接引用。
param([switch]$SkipBuild)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

# 版本号只认 workspace Cargo.toml：nsi 的 APP_VERSION 由这里通过 /D 传入，
# 避免发版时改了 Cargo.toml 忘了同步安装包版本（exe 版本信息在 assets\neverwork.rc，仍需同步改）。
$ver = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $ver) { throw "无法从 Cargo.toml 读取版本号" }
Write-Host "NeverWork $ver"

$running = Get-Process neverwork -ErrorAction SilentlyContinue
if ($running) {
    throw "NeverWork 正在运行：先关闭再构建（进程占用 exe 时 cargo 会失败且返回码仍是 0）。"
}

if (-not $SkipBuild) {
    Push-Location $root
    try {
        cargo build --release -p neverwork
        if ($LASTEXITCODE -ne 0) { throw "cargo build --release 失败" }
    } finally {
        Pop-Location
    }
}

$makensis = @(
    (Join-Path ${env:ProgramFiles(x86)} "NSIS\makensis.exe"),
    (Join-Path $env:ProgramFiles "NSIS\makensis.exe")
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $makensis) {
    $cmd = Get-Command makensis -ErrorAction SilentlyContinue
    if ($cmd) { $makensis = $cmd.Source }
}
if (-not $makensis) {
    throw "未找到 makensis.exe：请先安装 NSIS（winget install NSIS.NSIS）"
}

New-Item -ItemType Directory -Force (Join-Path $root "dist") | Out-Null
Push-Location $PSScriptRoot
try {
    & $makensis "/DAPP_VERSION=$ver" "neverwork.nsi"
    if ($LASTEXITCODE -ne 0) { throw "makensis 编译失败" }
} finally {
    Pop-Location
}

Get-Item (Join-Path $root "dist\NeverWork-$ver-setup.exe") |
    Select-Object FullName, Length, LastWriteTime
