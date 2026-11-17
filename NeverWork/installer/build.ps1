# 构建 NeverWork release 可执行文件并打出 NSIS 安装包。
#
# 用法（从仓库任意位置）：
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1            # 全流程
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1 -SkipBuild # 复用已有 release
#
# 依赖：Rust 工具链、Python + Pillow（生成图标）、NSIS（winget install NSIS.NSIS）
param([switch]$SkipBuild)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

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

python "$PSScriptRoot\make_icon.py"
if ($LASTEXITCODE -ne 0) { throw "图标生成失败（需要 Python + Pillow）" }

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
    & $makensis "neverwork.nsi"
    if ($LASTEXITCODE -ne 0) { throw "makensis 编译失败" }
} finally {
    Pop-Location
}

Get-Item (Join-Path $root "dist\NeverWork-0.1.0-setup.exe") |
    Select-Object FullName, Length, LastWriteTime
