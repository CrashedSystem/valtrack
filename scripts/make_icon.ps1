# Builds res/app.ico (multi-size, PNG-compressed entries) from a drawn emblem.
# Kept separate from the agent/rank art so the exe/shortcut/tray icon is stable
# even when the asset snapshot is refreshed.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$outDir = Join-Path $PSScriptRoot "..\res"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

function New-Emblem([int]$size) {
  $bmp = New-Object System.Drawing.Bitmap $size, $size
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
  $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAlias
  $g.Clear([System.Drawing.Color]::Transparent)

  # Rounded dark tile.
  $pad = [single]($size * 0.04)
  $rect = New-Object System.Drawing.RectangleF $pad, $pad, ($size - 2 * $pad), ($size - 2 * $pad)
  $r = [single]($size * 0.22)
  $path = New-Object System.Drawing.Drawing2D.GraphicsPath
  $path.AddArc($rect.X, $rect.Y, $r, $r, 180, 90)
  $path.AddArc($rect.Right - $r, $rect.Y, $r, $r, 270, 90)
  $path.AddArc($rect.Right - $r, $rect.Bottom - $r, $r, $r, 0, 90)
  $path.AddArc($rect.X, $rect.Bottom - $r, $r, $r, 90, 90)
  $path.CloseFigure()

  $bg = New-Object System.Drawing.Drawing2D.LinearGradientBrush $rect, ([System.Drawing.Color]::FromArgb(255, 34, 26, 30)), ([System.Drawing.Color]::FromArgb(255, 22, 18, 20)), 90
  $g.FillPath($bg, $path)

  # Red accent bar at the top.
  $accent = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 255, 70, 85))
  $g.FillRectangle($accent, $rect.X, $rect.Y, $rect.Width, [single]($size * 0.09))

  # Crosshair: outer ring + centre dot.
  $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 240, 240, 240)), ([single][Math]::Max(1.2, $size * 0.055))
  $cx = $rect.X + $rect.Width / 2
  $cy = $rect.Y + $rect.Height / 2 + $size * 0.03
  $rad = $size * 0.21
  $g.DrawEllipse($pen, $cx - $rad, $cy - $rad, 2 * $rad, 2 * $rad)
  $g.DrawLine($pen, $cx - $rad * 1.45, $cy, $cx - $rad * 0.55, $cy)
  $g.DrawLine($pen, $cx + $rad * 0.55, $cy, $cx + $rad * 1.45, $cy)
  $g.DrawLine($pen, $cx, $cy - $rad * 1.45, $cx, $cy - $rad * 0.55)
  $g.DrawLine($pen, $cx, $cy + $rad * 0.55, $cx, $cy + $rad * 1.45)
  $g.FillEllipse((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 255, 70, 85))), $cx - $size * 0.045, $cy - $size * 0.045, $size * 0.09, $size * 0.09)

  $g.Dispose()
  return $bmp
}

$sizes = @(16, 32, 48, 64, 128, 256)
$pngs = @()
foreach ($s in $sizes) {
  $bmp = New-Emblem $s
  $ms = New-Object System.IO.MemoryStream
  $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
  $pngs += , $ms.ToArray()
  $bmp.Dispose()
  $ms.Dispose()
}

# ICO container: header + 16-byte directory entries + PNG payloads.
$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter $ms
$bw.Write([UInt16]0)            # reserved
$bw.Write([UInt16]1)            # type: icon
$bw.Write([UInt16]$sizes.Count) # image count

$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
  $s = $sizes[$i]
  $bw.Write([Byte]$(if ($s -ge 256) { 0 } else { $s }))
  $bw.Write([Byte]$(if ($s -ge 256) { 0 } else { $s }))
  $bw.Write([Byte]0)   # palette
  $bw.Write([Byte]0)   # reserved
  $bw.Write([UInt16]1) # planes
  $bw.Write([UInt16]32)
  $bw.Write([UInt32]$pngs[$i].Length)
  $bw.Write([UInt32]$offset)
  $offset += $pngs[$i].Length
}
foreach ($p in $pngs) { $bw.Write($p) }
$bw.Flush()

$ico = Join-Path $outDir "app.ico"
[System.IO.File]::WriteAllBytes($ico, $ms.ToArray())
$bw.Dispose(); $ms.Dispose()

"icon: $ico (" + (Get-Item $ico).Length + " bytes, " + $sizes.Count + " sizes)"
