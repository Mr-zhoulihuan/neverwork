Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class W {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int ht, bool repaint);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  public struct RECT { public int L,T,R,B; }
}
"@
$procs = Get-Process neverwork -ErrorAction SilentlyContinue
if (-not $procs) { Write-Output "no process"; exit 1 }
$pids = $procs | ForEach-Object { [uint32]$_.Id }
$found = @()
$cb = [W+EnumWindowsProc]{
  param($h,$l)
  $pid2 = 0
  [W]::GetWindowThreadProcessId($h, [ref]$pid2) | Out-Null
  if ($pids -contains $pid2 -and [W]::IsWindowVisible($h)) {
    $r = New-Object W+RECT
    [W]::GetWindowRect($h, [ref]$r) | Out-Null
    $sz = "$($r.R-$r.L)x$($r.B-$r.T)"
    if ($sz -ne "0x0") { $script:found += [pscustomobject]@{H=$h; W=($r.R-$r.L); Ht=($r.B-$r.T)} }
  }
  return $true
}
[W]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null
if ($found.Count -eq 0) { Write-Output "no visible window"; exit 1 }
$w = $found | Sort-Object W -Descending | Select-Object -First 1
Write-Output "window hwnd=$($w.H) size=$($w.W)x$($w.Ht)"
# 移到左上角并置前
[W]::MoveWindow($w.H, 0, 0, $w.W, $w.Ht, $true) | Out-Null
[W]::SetForegroundWindow($w.H) | Out-Null
Write-Output "moved to 0,0"
