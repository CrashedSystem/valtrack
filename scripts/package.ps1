# Packages what a release needs: both binaries (app + GUI installer), README and
# LICENSE, plus a SHA-256 checksum someone can verify before running an unsigned
# binary.
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

# Both binaries ship together: setup copies valtrack.exe from its own directory,
# so the two must stay side by side in the zip.
$bins = @("valtrack.exe", "valtrack-setup.exe")
foreach ($name in $bins) {
  $p = Join-Path $repo "dist\$name"
  if (-not (Test-Path $p)) {
    throw "missing $p - run scripts/build.sh first"
  }
}

$stage = Join-Path $env:TEMP ("valtrack-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $stage | Out-Null
try {
  foreach ($name in $bins) {
    Copy-Item (Join-Path $repo "dist\$name") (Join-Path $stage $name)
  }
  foreach ($name in "README.md", "LICENSE") {
    $p = Join-Path $repo $name
    if (-not (Test-Path $p)) { throw "missing $p" }
    Copy-Item $p (Join-Path $stage $name)
  }

  $outDir = Join-Path $repo "dist\release"
  New-Item -ItemType Directory -Force -Path $outDir | Out-Null
  $stem = "valtrack-v$Version-win64"
  $zip = Join-Path $outDir "$stem.zip"
  Remove-Item $zip -Force -ErrorAction SilentlyContinue
  Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip -CompressionLevel Optimal -Force

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
