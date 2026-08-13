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

function Stage-PinnedFlect {
    param($Source, [string]$RuntimeRoot)

    $staged = Join-Path $RuntimeRoot 'flect.exe'
    [System.IO.Directory]::CreateDirectory($RuntimeRoot) | Out-Null
    Copy-Item -LiteralPath $Source.path -Destination $staged -Force
    $sourceHash = (Get-FileHash -LiteralPath $Source.path -Algorithm SHA256).Hash
    $stagedHash = (Get-FileHash -LiteralPath $staged -Algorithm SHA256).Hash
    if ($stagedHash -ne $sourceHash) {
        throw "Staged Flect hash mismatch: expected $sourceHash, got $stagedHash"
    }
    Get-PinnedFlect $staged
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

function Get-TypedCommandGrammar {
    param($Pinned)

    $specs = @(
        @{ name = 'verifier-begin'; syntax = "<flect> --json agent verifier-begin --job <blind-job-id> --model <model> --model-selection <explicit|inherited|unknown>"; required = @('--job', '--model', '--model-selection') },
        @{ name = 'verifier-set-objective'; syntax = "<flect> --json agent verifier-set-objective --job <blind-job-id> --text-file <utf8-text-path>"; required = @('--job', '--text-file') },
        @{ name = 'verifier-add-before'; syntax = "<flect> --json agent verifier-add-before --job <blind-job-id> --text-file <utf8-text-path>"; required = @('--job', '--text-file') },
        @{ name = 'verifier-add-after'; syntax = "<flect> --json agent verifier-add-after --job <blind-job-id> --text-file <utf8-text-path>"; required = @('--job', '--text-file') },
        @{ name = 'verifier-add-scope'; syntax = "<flect> --json agent verifier-add-scope --job <blind-job-id> --file <allowed-bundle-path> [--symbol-file <utf8-text-path>]"; required = @('--job', '--file') },
        @{ name = 'verifier-add-side-effect'; syntax = "<flect> --json agent verifier-add-side-effect --job <blind-job-id> --text-file <utf8-text-path>"; required = @('--job', '--text-file') },
        @{ name = 'verifier-add-assumption'; syntax = "<flect> --json agent verifier-add-assumption --job <blind-job-id> --text-file <utf8-text-path>"; required = @('--job', '--text-file') },
        @{ name = 'verifier-add-uncertainty'; syntax = "<flect> --json agent verifier-add-uncertainty --job <blind-job-id> --text-file <utf8-text-path>"; required = @('--job', '--text-file') },
        @{ name = 'verifier-set-confidence'; syntax = "<flect> --json agent verifier-set-confidence --job <blind-job-id> <finite-confidence-0..1>"; required = @('--job', '<CONFIDENCE>') },
        @{ name = 'verifier-submit'; syntax = "<flect> --json agent verifier-submit --job <blind-job-id>"; required = @('--job') },
        @{ name = 'judge-begin'; syntax = "<flect> --json agent judge-begin --job <judge-job-id> --model <model> --model-selection <explicit|inherited|unknown>"; required = @('--job', '--model', '--model-selection') },
        @{ name = 'judge-set-alignment'; syntax = "<flect> --json agent judge-set-alignment --job <judge-job-id> <SAME|PARTIAL|DIFFERENT|UNCERTAIN>"; required = @('--job', '<ALIGNMENT>') },
        @{ name = 'judge-add-finding'; syntax = "<flect> --json agent judge-add-finding --job <judge-job-id> --kind <finding-kind> --text-file <utf8-text-path> [--evidence-ref <hunk-id>]"; required = @('--job', '--kind', '--text-file') },
        @{ name = 'judge-add-side-effect-finding'; syntax = "<flect> --json agent judge-add-side-effect-finding --job <judge-job-id> --candidate <side_effect/n> --text-file <utf8-text-path> --evidence-ref <hunk-id>"; required = @('--job', '--candidate', '--text-file', '--evidence-ref') },
        @{ name = 'judge-mark-side-effect-not-distinct'; syntax = "<flect> --json agent judge-mark-side-effect-not-distinct --job <judge-job-id> --candidate <side_effect/n> --reason-file <utf8-text-path>"; required = @('--job', '--candidate', '--reason-file') },
        @{ name = 'judge-set-confidence'; syntax = "<flect> --json agent judge-set-confidence --job <judge-job-id> <finite-confidence-0..1>"; required = @('--job', '<CONFIDENCE>') },
        @{ name = 'judge-submit'; syntax = "<flect> --json agent judge-submit --job <judge-job-id>"; required = @('--job') }
    )
    foreach ($spec in $specs) {
        $help = (& $Pinned.path agent $spec.name --help) -join "`n"
        if ($LASTEXITCODE -ne 0) { throw "Could not read pinned CLI grammar for $($spec.name)" }
        if ($help -notmatch '(?m)^\s+Usage:') { throw "Missing usage grammar for $($spec.name)" }
        foreach ($token in $spec.required) {
            if ($help -notmatch [regex]::Escape($token)) { throw "Pinned CLI help for $($spec.name) lacks required token $token" }
        }
        $spec.help = $help
    }
    return $specs
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
    param([string]$Path, $Pinned, [string]$JobId, [string]$AgentTempRoot, [string]$CasePath, $Grammar)

    $manifest = [pscustomobject]@{
        protocol = 'typed-native-flect'
        flect_path = $Pinned.path
        flect_sha256 = $Pinned.sha256
        flect_version = $Pinned.version
        agent_temp_root = $AgentTempRoot
        case_path = $CasePath
        blind_job_id = $JobId
        verifier_draft_root = (Join-Path $AgentTempRoot ("flect-agent-jobs\$JobId\draft"))
        judge_draft_root_pattern = (Join-Path $AgentTempRoot 'flect-agent-jobs\<judge-job-id>\draft')
        verifier_protocol = @('objective', 'confidence', 'model', 'model_selection', 'behavior_before/000000.txt', 'behavior_after/000000.txt', 'affected_scope/000000/file', 'affected_scope/000000/symbol', 'side_effects/000000.txt', 'assumptions/000000.txt', 'uncertainties/000000.txt', 'submitted')
        judge_protocol = @('alignment/<SAME|PARTIAL|DIFFERENT|UNCERTAIN>', 'confidence', 'model', 'model_selection', 'findings/000000/<kind>', 'findings/000000/text', 'findings/000000/evidence_ref', 'side_effect_dispositions/side_effect/<n>/{finding|not-distinct}', 'submitted')
        rules = @('Do not execute Flect or any repository command.', 'Write only primitive UTF-8 draft files in the generated draft roots.', 'Write non-marker values with no BOM and no trailing CR/LF using an exact-byte API.', 'Scalar names are extensionless; numbered entries are zero-based, consecutive, and six digits.', 'Write actual runtime model and explicit|inherited|unknown selection.', 'Create submitted last as exactly zero bytes and verify its length.', 'Do not write JSON, use chat protocol, retry, repair, normalize, or infer semantics.')
    }
    Write-Utf8File $Path ($manifest | ConvertTo-Json -Depth 20)
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

$sourcePinned = Get-PinnedFlect $FlectBinary
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
$pinned = Stage-PinnedFlect $sourcePinned (Join-Path $ControllerRoot 'runtime')
$grammar = Get-TypedCommandGrammar $pinned

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
    Write-AgentDispatchInstructions (Join-Path $controlPath 'dispatch-manifest.json') $pinned $blindJob.job_id $agentTempRoot $casePath $grammar
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
