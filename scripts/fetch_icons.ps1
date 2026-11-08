$OutputEncoding=[Console]::OutputEncoding=[Text.Encoding]::UTF8
Add-Type -AssemblyName System.Drawing
$base = Join-Path $PSScriptRoot "..\assets"
New-Item -ItemType Directory -Force -Path "$base\agents","$base\ranks" | Out-Null

function Save-Scaled($url, $out, $size) {
  $tmp = Join-Path $env:TEMP ("icon_" + [guid]::NewGuid().ToString("N") + ".png")
  & curl.exe -s -o $tmp $url
  if(-not (Test-Path $tmp)) { return $false }
  try {
    $img = [System.Drawing.Image]::FromFile($tmp)
    $bmp = New-Object System.Drawing.Bitmap $size,$size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.DrawImage($img, 0, 0, $size, $size)
    $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose(); $img.Dispose()
    return $true
  } catch {
    return $false
  } finally {
    Remove-Item $tmp -ErrorAction SilentlyContinue
  }
}

$agents = Invoke-RestMethod "https://valorant-api.com/v1/agents?isPlayableCharacter=true"
$n = 0
foreach($a in $agents.data){
  if(-not $a.displayIcon){ continue }
  if(Save-Scaled $a.displayIcon "$base\agents\$($a.uuid).png" 64){ $n++ }
}
"agents saved: $n"

$tiers = Invoke-RestMethod "https://valorant-api.com/v1/competitivetiers"
$set = $tiers.data[-1]
$m = 0
foreach($t in $set.tiers){
  $url = if($t.smallIcon){ $t.smallIcon } elseif($t.largeIcon){ $t.largeIcon } else { $null }
  if(-not $url){ continue }
  if(Save-Scaled $url "$base\ranks\$($t.tier).png" 32){ $m++ }
}
"ranks saved: $m"
"total bytes: " + ((Get-ChildItem $base -Recurse -File | Measure-Object Length -Sum).Sum / 1KB) + " KB"
