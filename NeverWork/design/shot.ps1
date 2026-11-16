# DPI-aware full-screen capture helper.
# PowerShell is DPI-unaware by default: CopyFromScreen then only grabs the
# top-left quarter of a 200%-scaled screen, which fakes an "UI is magnified /
# right side cut off" symptom. Declare per-monitor awareness first.
# -Crop "x,y,w,h" (logical px) captures only that region; empty = full screen.
param(
  [string]$OutPath = "E:\AI\ide\NeverWork\design\run.png",
  [int]$ScreenW = 2560,
  [int]$ScreenH = 1600,
  [int]$Scale = 2,
  [string]$Crop = ""
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
$srcX = 0; $srcY = 0; $srcW = $ScreenW; $srcH = $ScreenH
if ($Crop -ne "") {
  $p = $Crop -split ","
  $srcX = [int]$p[0] * $Scale
  $srcY = [int]$p[1] * $Scale
  $srcW = [int]$p[2] * $Scale
  $srcH = [int]$p[3] * $Scale
  $srcW = [Math]::Min($srcW, $ScreenW - $srcX)
  $srcH = [Math]::Min($srcH, $ScreenH - $srcY)
}
$small = New-Object System.Drawing.Bitmap ([int]($srcW / $Scale)), ([int]($srcH / $Scale))
$g2 = [System.Drawing.Graphics]::FromImage($small)
$g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$dest = New-Object System.Drawing.Rectangle 0, 0, $small.Width, $small.Height
$src = New-Object System.Drawing.Rectangle $srcX, $srcY, $srcW, $srcH
$g2.DrawImage($full, $dest, $src, [System.Drawing.GraphicsUnit]::Pixel)
$small.Save($OutPath, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $g2.Dispose(); $full.Dispose(); $small.Dispose()
Write-Output "saved $OutPath ($([int]($srcW / $Scale))x$([int]($srcH / $Scale)))"
