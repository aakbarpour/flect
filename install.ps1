[CmdletBinding()]
param(
    [string]$Version = 'latest',
    [string]$BinDir,
    [string]$Archive,
    [string]$ChecksumFile
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    throw "Flect installer: $Message"
}

if ([string]::IsNullOrWhiteSpace($BinDir)) {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        Fail 'LOCALAPPDATA is not set; pass -BinDir explicitly.'
    }
    $BinDir = Join-Path $env:LOCALAPPDATA 'Flect\bin'
}

if ([string]::IsNullOrWhiteSpace($Archive) -xor [string]::IsNullOrWhiteSpace($ChecksumFile)) {
    Fail '-Archive and -ChecksumFile must be supplied together.'
}

if ($Version -ne 'latest' -and $Version -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') {
    Fail '-Version must be latest or vX.Y.Z.'
}

$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
if ($architecture -ne 'X64') {
    Fail "unsupported Windows architecture: $architecture"
}
$target = 'x86_64-pc-windows-msvc'

function Assert-RegularFile([string]$Path, [string]$Description) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Fail "$Description does not exist: $Path"
    }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        Fail "$Description must not be a symlink or reparse point: $Path"
    }
}

function Invoke-HttpsDownload([string]$Url, [string]$Destination) {
    if (-not $Url.StartsWith('https://', [System.StringComparison]::OrdinalIgnoreCase)) {
        Fail "refusing non-HTTPS download URL: $Url"
    }
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    try {
        Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
    }
    catch {
        Fail "download failed: $Url ($($_.Exception.Message))"
    }
}

function Get-ExpectedChecksum([string]$Path, [string]$ExpectedName) {
    foreach ($line in Get-Content -LiteralPath $Path) {
        if ($line -match '^\s*([0-9A-Fa-f]{64})\s+\*?(.*?)\s*$') {
            $candidate = $Matches[2].Trim()
            if ($candidate -eq $ExpectedName) {
                return $Matches[1].ToLowerInvariant()
            }
        }
    }
    Fail "checksum entry for $ExpectedName was not found"
}

function Assert-SafeZip([string]$Path, [string]$RootName) {
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $required = @("$RootName/flect.exe", "$RootName/LICENSE", "$RootName/README.md")
        $names = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($entry in $zip.Entries) {
            $normalized = $entry.FullName.Replace('\', '/')
            if ([IO.Path]::IsPathRooted($normalized) -or $normalized.StartsWith('/') -or
                ($normalized -split '/') -contains '..') {
                Fail "unsafe archive entry: $($entry.FullName)"
            }
            if (-not $entry.FullName.EndsWith('/')) {
                [void]$names.Add($normalized)
            }
        }
        foreach ($name in $required) {
            if (-not $names.Contains($name)) {
                Fail "archive does not contain required entry: $name"
            }
        }
    }
    finally {
        $zip.Dispose()
    }
}

$workDir = Join-Path ([IO.Path]::GetTempPath()) ('flect-install-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $workDir -Force | Out-Null
try {
    if (-not [string]::IsNullOrWhiteSpace($Archive)) {
        Assert-RegularFile $Archive 'archive'
        Assert-RegularFile $ChecksumFile 'checksum file'
        $archivePath = [IO.Path]::GetFullPath($Archive)
        $checksumPath = [IO.Path]::GetFullPath($ChecksumFile)
        $archiveName = [IO.Path]::GetFileName($archivePath)
        if ($archiveName -notmatch "^flect-.+-$([regex]::Escape($target))\.zip$") {
            Fail "archive does not match target $target`: $archiveName"
        }
        if ($Version -ne 'latest' -and $archiveName -ne "flect-$Version-$target.zip") {
            Fail "archive does not match requested version $Version"
        }
    }
    else {
        $archiveName = "flect-$Version-$target.zip"
        if ($Version -eq 'latest') {
            $releaseUrl = 'https://github.com/aakbarpour/flect/releases/latest/download'
        }
        else {
            $releaseUrl = "https://github.com/aakbarpour/flect/releases/download/$Version"
        }
        $archivePath = Join-Path $workDir $archiveName
        $checksumPath = Join-Path $workDir 'SHA256SUMS'
        Invoke-HttpsDownload "$releaseUrl/$archiveName" $archivePath
        Invoke-HttpsDownload "$releaseUrl/SHA256SUMS" $checksumPath
    }

    $expected = Get-ExpectedChecksum $checksumPath $archiveName
    $actual = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        Fail "SHA256 checksum mismatch for $archiveName"
    }

    $archiveRoot = $archiveName.Substring(0, $archiveName.Length - 4)
    Assert-SafeZip $archivePath $archiveRoot
    $extractDir = Join-Path $workDir 'extracted'
    New-Item -ItemType Directory -Path $extractDir -Force | Out-Null
    [IO.Compression.ZipFile]::ExtractToDirectory($archivePath, $extractDir)
    $binaryPath = Join-Path $extractDir "$archiveRoot\flect.exe"
    Assert-RegularFile $binaryPath 'archive executable'
    foreach ($requiredFile in @('LICENSE', 'README.md')) {
        Assert-RegularFile (Join-Path $extractDir "$archiveRoot\$requiredFile") "archive $requiredFile"
    }

    $binDirFull = [IO.Path]::GetFullPath($BinDir)
    if (Test-Path -LiteralPath $binDirFull) {
        $binItem = Get-Item -LiteralPath $binDirFull -Force
        if (($binItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Fail "installation directory must not be a symlink or reparse point: $binDirFull"
        }
    }
    New-Item -ItemType Directory -Path $binDirFull -Force | Out-Null
    $temporaryBinary = Join-Path $workDir 'flect.exe'
    Copy-Item -LiteralPath $binaryPath -Destination $temporaryBinary -Force
    $destination = Join-Path $binDirFull 'flect.exe'
    if (Test-Path -LiteralPath $destination) {
        $destinationItem = Get-Item -LiteralPath $destination -Force
        if ($destinationItem.PSIsContainer -or
            (($destinationItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
            Fail "installation destination must be a regular file: $destination"
        }
    }
    Move-Item -LiteralPath $temporaryBinary -Destination $destination -Force

    Write-Host "Installed Flect $archiveRoot for $target to $destination"
    $pathEntries = @($env:Path -split ';' | Where-Object { $_ -ne '' } | ForEach-Object {
        try { [IO.Path]::GetFullPath($_) } catch { $null }
    } | Where-Object { $_ -ne $null })
    if ($pathEntries -notcontains $binDirFull) {
        Write-Host "$binDirFull is not currently on PATH. Add it to this PowerShell session with:"
        Write-Host ('$env:Path = "' + $binDirFull + ';$env:Path"')
    }
}
finally {
    if (Test-Path -LiteralPath $workDir) {
        Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
