[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$FlectBinary,

    [string]$SuitePath,

    [string]$CaseRoot,

    [string]$ControllerRoot,

    [string]$RunId
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($SuitePath)) {
    $SuitePath = Join-Path $PSScriptRoot "..\fixtures\evaluation\cases.json"
}
if ([string]::IsNullOrWhiteSpace($CaseRoot)) {
    $CaseRoot = Join-Path $PSScriptRoot "..\canonical-native-clean-1f20bb3a"
}
if ([string]::IsNullOrWhiteSpace($ControllerRoot)) {
    $ControllerRoot = Join-Path ([System.IO.Path]::GetTempPath()) "flect-canonical-controller-1f20bb3a"
}
$SuitePath = [System.IO.Path]::GetFullPath($SuitePath)
$CaseRoot = [System.IO.Path]::GetFullPath($CaseRoot)
$ControllerRoot = [System.IO.Path]::GetFullPath($ControllerRoot)
$FlectBinary = [System.IO.Path]::GetFullPath($FlectBinary)
if ([string]::IsNullOrWhiteSpace($RunId)) {
    $RunId = "canonical-$(Get-Date -Format 'yyyyMMddHHmmss')-$([Guid]::NewGuid().ToString('N'))"
}

function Get-PinnedFlect {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Pinned Flect executable does not exist: $Path"
    }
    $help = (& $Path agent --help) -join "`n"
    foreach ($command in @('verifier-begin', 'verifier-submit', 'judge-begin', 'judge-submit')) {
        if ($help -notmatch "(?m)^\s+$command\s") {
            throw "Pinned Flect executable lacks required typed command '$command': $Path"
        }
    }
    [pscustomobject]@{
        path = $Path
        sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
        version = ((& $Path --version) -join "`n").Trim()
    }
}

function Assert-PinnedFlect {
    param($Pinned)

    $actual = (Get-FileHash -LiteralPath $Pinned.path -Algorithm SHA256).Hash
    if ($actual -ne $Pinned.sha256) {
        throw "Pinned Flect executable hash changed: expected $($Pinned.sha256), got $actual"
    }
    $help = (& $Pinned.path agent --help) -join "`n"
    foreach ($command in @('verifier-begin', 'verifier-submit', 'judge-begin', 'judge-submit')) {
        if ($help -notmatch "(?m)^\s+$command\s") {
            throw "Pinned Flect executable no longer exposes required typed command '$command'"
        }
    }
}

function Write-Utf8File {
    param([string]$Path, [string]$Content)

    $parent = Split-Path -Parent $Path
    [System.IO.Directory]::CreateDirectory($parent) | Out-Null
    [System.IO.File]::WriteAllText($Path, $Content, [System.Text.UTF8Encoding]::new($false))
}

function Invoke-CaseCommand {
    param([string]$CasePath, [string]$Program, [string[]]$Arguments, [string]$AgentTempRoot)

    Push-Location $CasePath
    try {
        $previousTemp = $env:TEMP
        $previousTmp = $env:TMP
        if (-not [string]::IsNullOrWhiteSpace($AgentTempRoot)) {
            $env:TEMP = $AgentTempRoot
            $env:TMP = $AgentTempRoot
        }
        & $Program @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "Command failed in ${CasePath}: $Program $($Arguments -join ' ')"
        }
    }
    finally {
        $env:TEMP = $previousTemp
        $env:TMP = $previousTmp
        Pop-Location
    }
}

function Assert-CaseStatus {
    param($Case, [string]$CasePath)

    $actual = @(git -C $CasePath status --porcelain --untracked-files=all)
    $expected = @($Case.candidate_patch.files | ForEach-Object { " M $($_.path)" })
    if (@($actual).Count -ne @($expected).Count -or (Compare-Object $actual $expected)) {
        throw "Candidate-status contamination in $($Case.id): expected only [$($expected -join ', ')], got [$($actual -join ', ')]"
    }
}

function Assert-NoControlFiles {
    param([string]$CasePath)

    $artifactName = '(?i)(intended[-_]?spec|expected[-_]?verdict|expected[-_]?categor|evaluator|ground[-_]?truth|benchmark[-_]?control|case\.json)'
    $artifactContent = '(?i)("intended_spec"|"expected_finding_categories"|"mock_verdict"|"mock_echoed_spec"|"original_task"|"expected"\s*:|evaluator metadata|benchmark ground truth)'
    $files = Get-ChildItem -LiteralPath $CasePath -Recurse -Force -File |
        Where-Object {
            $relative = $_.FullName.Substring($CasePath.Length).TrimStart('\', '/')
            -not ($relative -like '.git\*' -or $relative -like '.flect\*')
        }
    foreach ($file in $files) {
        if ($file.Name -match $artifactName) {
            throw "Benchmark-control path leaked into case repository: $($file.FullName)"
        }
        $text = [System.IO.File]::ReadAllText($file.FullName)
        if ($text -match $artifactContent) {
            throw "Benchmark-control content leaked into case repository: $($file.FullName)"
        }
    }
}

function Assert-CleanBlindBundle {
    param([string]$BlindJobPath, [string]$ControllerPath)

    $job = [System.IO.File]::ReadAllText($BlindJobPath) | ConvertFrom-Json
    $raw = $job.bundle | ConvertTo-Json -Depth 100
    $artifactPattern = '(?i)(intended[-_]?spec|expected[-_]?verdict|expected[-_]?categor|evaluator|ground[-_]?truth|benchmark[-_]?control|case\.json|"intended_spec"|"expected_finding_categories"|"mock_verdict"|"mock_echoed_spec"|"original_task")'
    if ($raw -match $artifactPattern -or $raw.Contains($ControllerPath)) {
        throw "BlindBundle contamination detected in $BlindJobPath"
    }
}

function Write-AgentDispatchInstructions {
    param([string]$Path, $Pinned, [string]$JobId, [string]$AgentTempRoot)

    Write-Utf8File $Path @"
Pinned Flect executable: $($Pinned.path)
Pinned SHA-256: $($Pinned.sha256)
External agent TEMP/TMP root: $AgentTempRoot
Blind job: $JobId

Before every Flect command, set both TEMP and TMP to the external agent root above and verify the executable hash equals the pinned SHA-256. Invoke only this absolute executable, never bare flect. Every verifier command must include the agent subcommand, for example:

& '$($Pinned.path)' --json agent verifier-begin --job '$JobId' --model gpt-5.6-terra --model-selection explicit

Continue only with the typed verifier lifecycle using that same prefix, then submit. Do not run prepare-blind: this is the sole prepared blind job for this case.

For the separate judge, keep the same pinned executable and SHA-256, but run the repository-scoped typed judge lifecycle from the case repository. The parent will provide the Flect-generated judge job ID and contract. Use:

& '$($Pinned.path)' --json agent judge-begin --job '<judge-job-id>' --model gpt-5.6-terra --model-selection explicit

Never invoke bare flect, omit the agent subcommand, or prepare another blind job.
"@
}

function New-CandidatePatch {
    param($Case)

    $lines = [System.Collections.Generic.List[string]]::new()
    foreach ($file in $Case.candidate_patch.files) {
        if ($file.status -ne 'modified' -or $file.binary) {
            throw "Canonical harness supports only text modified patches; $($Case.id) has $($file.path)"
        }
        $lines.Add("diff --git a/$($file.path) b/$($file.path)")
        $lines.Add("--- a/$($file.path)")
        $lines.Add("+++ b/$($file.path)")
        $lines.Add($file.patch)
    }
    return (($lines -join "`n") + "`n")
}

$pinned = Get-PinnedFlect $FlectBinary
$suite = Get-Content -LiteralPath $SuitePath -Raw | ConvertFrom-Json
$cases = @($suite.cases | Where-Object { $_.subset -eq 'canonical-5' })
if ($cases.Count -ne 5) {
    throw "Expected five canonical cases, found $($cases.Count)"
}

if (Test-Path -LiteralPath $CaseRoot) {
    Remove-Item -LiteralPath $CaseRoot -Recurse -Force
}
if (Test-Path -LiteralPath $ControllerRoot) {
    Remove-Item -LiteralPath $ControllerRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $CaseRoot, $ControllerRoot | Out-Null

$summary = @()
foreach ($case in $cases) {
    $casePath = Join-Path $CaseRoot $case.id
    $controlPath = Join-Path $ControllerRoot $case.id
    $agentTempRoot = Join-Path $controlPath 'agent-temp'
    $agentStateRoot = Join-Path $agentTempRoot 'flect-agent-jobs'
    New-Item -ItemType Directory -Path $casePath, $controlPath | Out-Null
    if (Test-Path -LiteralPath $agentStateRoot) {
        throw "Refusing pre-existing external agent state for $($case.id): $agentStateRoot"
    }
    New-Item -ItemType Directory -Path $agentTempRoot | Out-Null

    # All forward data remains outside the isolated case repository.
    Write-Utf8File (Join-Path $controlPath 'case.json') ($case | ConvertTo-Json -Depth 100)
    Write-Utf8File (Join-Path $controlPath 'intended-spec.json') ($case.intended_spec | ConvertTo-Json -Depth 100)
    $candidatePatchPath = Join-Path $controlPath 'candidate.patch'
    Write-Utf8File $candidatePatchPath (New-CandidatePatch $case)

    foreach ($baseFile in $case.base_files) {
        Write-Utf8File (Join-Path $casePath $baseFile.path) $baseFile.content
    }

    Invoke-CaseCommand $casePath 'git' @('init', '-b', 'main') $null
    Invoke-CaseCommand $casePath 'git' @('config', 'user.email', 'benchmark@flect.local') $null
    Invoke-CaseCommand $casePath 'git' @('config', 'user.name', 'Flect Canonical Harness') $null
    Invoke-CaseCommand $casePath 'git' @('add', '--all') $null
    Invoke-CaseCommand $casePath 'git' @('commit', '--quiet', '-m', 'canonical fixture base') $null
    Assert-PinnedFlect $pinned
    Invoke-CaseCommand $casePath $pinned.path @('init') $agentTempRoot
    Invoke-CaseCommand $casePath 'git' @('add', '.gitignore', 'flect.toml') $null
    Invoke-CaseCommand $casePath 'git' @('commit', '--quiet', '-m', 'flect runtime configuration') $null
    Assert-PinnedFlect $pinned
    Invoke-CaseCommand $casePath $pinned.path @('start', '--task', $case.original_task, '--spec-file', (Join-Path $controlPath 'intended-spec.json')) $agentTempRoot

    # Apply and validate the exact controller-owned candidate bytes before dispatch.
    Invoke-CaseCommand $casePath 'git' @('apply', '--check', $candidatePatchPath) $null
    $patchHash = (Get-FileHash -LiteralPath $candidatePatchPath -Algorithm SHA256).Hash
    Invoke-CaseCommand $casePath 'git' @('apply', '--whitespace=nowarn', $candidatePatchPath) $null
    Invoke-CaseCommand $casePath 'git' @('apply', '--check', '--reverse', $candidatePatchPath) $null
    Invoke-CaseCommand $casePath 'git' @('diff', '--check') $null

    Assert-CaseStatus $case $casePath
    Assert-NoControlFiles $casePath

    $blindJobPath = Join-Path $controlPath 'blind-job.json'
    Push-Location $casePath
    try {
        $previousTemp = $env:TEMP
        $previousTmp = $env:TMP
        $env:TEMP = $agentTempRoot
        $env:TMP = $agentTempRoot
        Assert-PinnedFlect $pinned
        $blind = & $pinned.path --json agent prepare-blind
        if ($LASTEXITCODE -ne 0) {
            throw "prepare-blind failed for $($case.id)"
        }
        Write-Utf8File $blindJobPath ($blind -join "`n")
    }
    finally {
        $env:TEMP = $previousTemp
        $env:TMP = $previousTmp
        Pop-Location
    }
    Assert-CleanBlindBundle $blindJobPath $ControllerRoot
    $blindJob = $blind | ConvertFrom-Json
    if ($blindJob.workspace -notlike "$agentStateRoot*") {
        throw "Blind job workspace is not in this case's fresh agent-state root: $($blindJob.workspace)"
    }
    Write-AgentDispatchInstructions (Join-Path $controlPath 'verifier-dispatch.md') $pinned $blindJob.job_id $agentTempRoot
    Write-Utf8File (Join-Path $controlPath 'run-manifest.json') ([pscustomobject]@{
        run_id = $RunId
        case_id = $case.id
        flect_path = $pinned.path
        flect_sha256 = $pinned.sha256
        flect_version = $pinned.version
        agent_temp_root = $agentTempRoot
        agent_state_root = $agentStateRoot
        blind_job_id = $blindJob.job_id
    } | ConvertTo-Json -Depth 10)

    $summary += [pscustomobject]@{
        case = $case.id
        status = ((git -C $casePath status --porcelain --untracked-files=all) -join '; ')
        candidate_patch_sha256 = $patchHash
        blind_job_id = $blindJob.job_id
        flect_sha256 = $pinned.sha256
        agent_state_root = $agentStateRoot
        blind_bundle = 'clean'
    }
}

$summary | ConvertTo-Json -Depth 10
