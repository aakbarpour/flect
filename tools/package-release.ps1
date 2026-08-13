param(
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = "Stop"
$binary = if ($Target -like "*-windows-*") { "flect.exe" } else { "flect" }
$name = "flect-$Version-$Target"
$dist = Join-Path $PSScriptRoot "..\dist"
$archive = Join-Path $dist "$name.zip"

if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
New-Item -ItemType Directory -Path $dist -Force | Out-Null

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::Open($archive, [IO.Compression.ZipArchiveMode]::Create)
try {
    $files = @(
        @{ Source = Join-Path $PSScriptRoot "..\target\$Target\release\$binary"; Name = $binary },
        @{ Source = Join-Path $PSScriptRoot "..\LICENSE"; Name = "LICENSE" },
        @{ Source = Join-Path $PSScriptRoot "..\README.md"; Name = "README.md" }
    )
    foreach ($file in $files) {
        $source = [IO.Path]::GetFullPath($file.Source)
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "missing package input $source"
        }
        $entryName = "$name/$($file.Name)"
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $zip,
            $source,
            $entryName,
            [IO.Compression.CompressionLevel]::Optimal
        ) | Out-Null
    }
}
finally {
    $zip.Dispose()
}
