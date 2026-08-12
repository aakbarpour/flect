param(
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = "Stop"
$binary = if ($Target -like "*-windows-*") { "flect.exe" } else { "flect" }
$name = "flect-$Version-$Target"
$dist = Join-Path $PSScriptRoot "..\dist"
$stage = Join-Path $dist $name
$archive = Join-Path $dist "$name.zip"

if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "..\target\$Target\release\$binary") -Destination (Join-Path $stage $binary)
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "..\LICENSE") -Destination $stage
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "..\README.md") -Destination $stage
Compress-Archive -LiteralPath $stage -DestinationPath $archive
Remove-Item -LiteralPath $stage -Recurse -Force

