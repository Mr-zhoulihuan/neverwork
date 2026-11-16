# 启动 neverwork 并按窗口矩形截图（DPI-aware）。
# 用法示例：
#   .\_run_shot.ps1 -Out design\m18-mcp-official.png -EnvPairs "NW_VIEW=skills;NW_MARKET_NO_REFRESH=0" -Wait 12
#   .\_run_shot.ps1 -Out design\m18-skill.png -EnvPairs "NW_VIEW=skills;NW_SKILLS_TAB=1" -Wait 12
param(
  [string]$Out = "E:\AI\ide\NeverWork\design\_shot.png",
  [string]$EnvPairs = "NW_VIEW=skills",
  [int]$Wait = 10,
  [int]$Max = 0
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32 {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
}
"@
[Win32]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

Get-Process neverwork -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400

Get-ChildItem Env: | Where-Object { $_.Name -like 'NW_*' } | ForEach-Object {
  Remove-Item -Path ("Env:" + $_.Name) -ErrorAction SilentlyContinue
}
foreach ($pair in $EnvPairs.Split(';')) {
  if ($pair.Trim() -eq '') { continue }
  $i = $pair.IndexOf('=')
  if ($i -lt 1) { continue }
  $k = $pair.Substring(0, $i).Trim()
  $v = $pair.Substring($i + 1)
  Set-Item -Path ("Env:" + $k) -Value $v
}
$env:NW_MAXIMIZE = "$Max"

$exe = "E:\AI\ide\NeverWork\target\debug\neverwork.exe"
cmd /c start /b "" "$exe"

$proc = $null
for ($i = 0; $i -lt 80; $i++) {
  Start-Sleep -Milliseconds 250
  $proc = Get-Process neverwork -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  if ($proc) { break }
}
if (-not $proc) { Write-Output "NO_WINDOW"; exit 1 }

[Win32]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Seconds $Wait

$r = New-Object Win32+RECT
[Win32]::GetWindowRect($proc.MainWindowHandle, [ref]$r) | Out-Null
$x = [int]($r.Left / 2); $y = [int]($r.Top / 2)
$w = [int](($r.Right - $r.Left) / 2); $h = [int](($r.Bottom - $r.Top) / 2)
$crop = "$x,$y,$w,$h"
& "E:\AI\ide\NeverWork\design\shot.ps1" -OutPath $Out -Crop $crop

Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Write-Output "DONE $Out"
