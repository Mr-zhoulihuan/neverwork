# DPI-aware full-screen capture helper.
# PowerShell is DPI-unaware by default: CopyFromScreen then only grabs the
# top-left quarter of a 200%-scaled screen, which fakes an "UI is magnified /
# right side cut off" symptom. Declare per-monitor awareness first.
param(
  [string]$OutPath = "E:\AI\ide\NeverWork\design\run.png",
  [int]$ScreenW = 2560,
  [int]$ScreenH = 1600,
  [int]$Scale = 2
)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class DpiFix {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
}
"@
[DpiFix]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
Add-Type -AssemblyName System.Drawing
$full = New-Object System.Drawing.Bitmap $ScreenW, $ScreenH
$g = [System.Drawing.Graphics]::FromImage($full)
$g.CopyFromScreen(0, 0, 0, 0, $full.Size)
$small = New-Object System.Drawing.Bitmap ([int]($ScreenW / $Scale)), ([int]($ScreenH / $Scale))
$g2 = [System.Drawing.Graphics]::FromImage($small)
$g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g2.DrawImage($full, 0, 0, $small.Width, $small.Height)
$small.Save($OutPath, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $g2.Dispose(); $full.Dispose(); $small.Dispose()
Write-Output "saved $OutPath ($([int]($ScreenW / $Scale))x$([int]($ScreenH / $Scale)))"
