# Packages what a release needs: a portable zip (exe + README + LICENSE) and a
# SHA-256 checksum users can verify before running an unsigned binary.
#
#   powershell -File scripts\package.ps1
#   powershell -File scripts\package.ps1 -Version 0.1.1
#
# Build first: wsl -d Ubuntu -e bash scripts/build.sh
param([string]$Version)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot

if (-not $Version) {
  $cargo = Get-Content (Join-Path $repo "Cargo.toml") -Raw
  if ($cargo -match '(?m)^version\s*=\s*"([^"]+)"') {
    $Version = $Matches[1]
  } else {
    throw "cannot read version from Cargo.toml"
  }
}

$exe = Join-Path $repo "dist\valtrack.exe"
if (-not (Test-Path $exe)) {
  throw "missing $exe - run scripts/build.sh first"
}

$stage = Join-Path $env:TEMP ("valtrack-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $stage | Out-Null
try {
  Copy-Item $exe (Join-Path $stage "valtrack.exe")
  foreach ($name in "README.md", "LICENSE") {
    $p = Join-Path $repo $name
    if (-not (Test-Path $p)) { throw "missing $p" }
    Copy-Item $p (Join-Path $stage $name)
  }

  $outDir = Join-Path $repo "dist\release"
  New-Item -ItemType Directory -Force -Path $outDir | Out-Null
  $stem = "valtrack-v$Version-win64"
  $zip = Join-Path $outDir "$stem.zip"
  Remove-Item $zip -ErrorAction SilentlyContinue
  Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip -CompressionLevel Optimal

  $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
  $sums = Join-Path $outDir "SHA256SUMS.txt"
  "$hash  $stem.zip" | Set-Content $sums -Encoding ascii

  $size = (Get-Item $zip).Length / 1MB
  "zip    $zip"
  "size   {0:N2} MB" -f $size
  "sha256 $hash"
  "sums   $sums"
} finally {
  Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
}
