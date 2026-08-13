[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string]$Manifest,
    [Parameter(Mandatory = $true)] [string]$CasePath,
    [Parameter(Mandatory = $true)] [string]$FlectArgumentsCsv
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$m = Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json
$FlectArguments = @($FlectArgumentsCsv -split ',')
$hash = (Get-FileHash -LiteralPath $m.flect_path -Algorithm SHA256).Hash
if ($hash -ne $m.flect_sha256) { throw "Pinned Flect hash mismatch" }
if ($m.flect_path -like '*target\debug\*' -or $m.flect_path -like '*target\release\*') { throw "Repository build-tree executable is forbidden" }
$oldTemp = $env:TEMP; $oldTmp = $env:TMP; $oldCwd = Get-Location
try {
    $env:TEMP = $m.agent_temp_root
    $env:TMP = $m.agent_temp_root
    Set-Location -LiteralPath $CasePath
    & $m.flect_path @FlectArguments
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
finally {
    $env:TEMP = $oldTemp; $env:TMP = $oldTmp; Set-Location $oldCwd
}
