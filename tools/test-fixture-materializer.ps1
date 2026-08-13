$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Import-Module (Join-Path $PSScriptRoot 'fixture-materializer.psm1') -Force

function Assert-Equal($Actual, $Expected, [string]$Message) {
    if ($Actual -cne $Expected) { throw "${Message}: expected [$Expected], got [$Actual]" }
}
function Assert-Throws([scriptblock]$Action, [string]$Message) {
    try { & $Action } catch { return }
    throw "Expected failure: $Message"
}
function New-Case([string]$Path, [string]$Patch) {
    [pscustomobject]@{
        id = 'fixture-test'
        base_files = @([pscustomobject]@{ path = $Path; content = "alpha`nbeta`n" })
        candidate_patch = [pscustomobject]@{
            files = @([pscustomobject]@{ path = $Path; status = 'modified'; patch = $Patch; insertions = 1; deletions = 1; binary = $false })
        }
    }
}

$root = Join-Path ([IO.Path]::GetTempPath()) ('flect-fixture-materializer-' + [Guid]::NewGuid().ToString('N'))
$control = Join-Path ([IO.Path]::GetTempPath()) ('flect-fixture-materializer-control-' + [Guid]::NewGuid().ToString('N'))
try {
    [IO.Directory]::CreateDirectory($root) | Out-Null
    [IO.Directory]::CreateDirectory($control) | Out-Null
    & git -C $root init -q
    & git -C $root config user.email fixture@test.local
    & git -C $root config user.name fixture-test
    & git -C $root config core.autocrlf false
    [IO.File]::WriteAllText((Join-Path $root 'item.txt'), "alpha`nbeta`n", [Text.UTF8Encoding]::new($false))
    & git -C $root add -- item.txt
    & git -C $root commit -qm base

    $valid = New-Case 'item.txt' "@@ -1,2 +1,2 @@`n alpha`n-beta`n+gamma"
    $validPatch = Join-Path $control 'valid.patch'
    $result = Invoke-FixtureMaterialization $root $valid $validPatch
    Assert-Equal $result.mode 'git_exact' 'valid native patch mode'
    Assert-FixtureMaterialization $root $valid
    & git -C $root restore --source=HEAD --staged --worktree -- item.txt

    $malformed = New-Case 'item.txt' "@@ -1 +1 @@`n-alpha`nbeta`n+alpha`ngamma"
    $malformedPatch = Join-Path $control 'malformed.patch'
    $result = Invoke-FixtureMaterialization $root $malformed $malformedPatch
    Assert-Equal $result.mode 'fixture_structural' 'malformed-count fixture mode'
    if ($result.hunk_count_metadata_valid) { throw 'malformed count was not reported' }
    Assert-FixtureMaterialization $root $malformed
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $root 'item.txt'))) "alpha`ngamma`n" 'structural content'
    & git -C $root restore --source=HEAD --staged --worktree -- item.txt

    $nonStandalone = Get-StructuralContent "alpha`nbeta`n" "@@ -99 +99 @@`n-alpha`nbeta`n+alpha`ngamma" 'item.txt'
    Assert-Equal $nonStandalone.content "alpha`ngamma`n" 'non-standalone structural patch'
    Assert-Throws { Get-StructuralContent "alpha`nalpha`n" "@@ -1 +1 @@`n-alpha`n+gamma" 'item.txt' } 'ambiguous old content'
    Assert-Throws { Get-StructuralContent "alpha`n" "@@ -1 +1 @@`n-beta`n+gamma" 'item.txt' } 'absent old content'

    [IO.File]::WriteAllText((Join-Path $root 'extra.txt'), 'unexpected', [Text.UTF8Encoding]::new($false))
    Assert-Throws { Assert-FixtureMaterialization $root $valid } 'extra file change'
    Remove-Item -LiteralPath (Join-Path $root 'extra.txt')
    'fixture materializer tests passed'
}
finally {
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
    if (Test-Path -LiteralPath $control) { Remove-Item -LiteralPath $control -Recurse -Force }
}
