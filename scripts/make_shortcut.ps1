# Puts valtrack on the Desktop as a plain .exe (no shortcut).
#
# A .lnk cannot be used reliably here: on this machine opening a shortcut
# launches Windows Terminal instead of the target. A real .exe double-clicked
# from the Desktop starts the overlay directly (no arguments = overlay).
$ErrorActionPreference = "Stop"

$src = Join-Path $PSScriptRoot "..\dist\valtrack.exe"
if (-not (Test-Path $src)) { throw "missing $src" }

$desktop = [Environment]::GetFolderPath("Desktop")
$dst = Join-Path $desktop "VALORANT Overlay.exe"
Copy-Item $src $dst -Force

# Drop anything left over from the shortcut approach.
Remove-Item (Join-Path $desktop "VALORANT Overlay.lnk") -ErrorAction SilentlyContinue
$korean = "VALORANT " + [char]0xC624 + [char]0xBC84 + [char]0xB808 + [char]0xC774
Remove-Item (Join-Path $desktop "$korean.lnk") -ErrorAction SilentlyContinue
Remove-Item (Join-Path $desktop "VTEST2.lnk") -ErrorAction SilentlyContinue
Remove-Item "$env:LOCALAPPDATA\valtrack" -Recurse -Force -ErrorAction SilentlyContinue

"placed: $dst"
