[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$FlectBinary
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$FlectBinary = [System.IO.Path]::GetFullPath($FlectBinary)
$root = Join-Path ([System.IO.Path]::GetTempPath()) ('flect-dispatch-smoke-' + [Guid]::NewGuid().ToString('N'))
$caseRoot = Join-Path $root 'cases'
$controllerRoot = Join-Path $root 'controller'
$case = Join-Path $caseRoot 'canonical-01'
$manifestPath = Join-Path $controllerRoot 'canonical-01\dispatch-manifest.json'
$previousTemp = $env:TEMP
$previousTmp = $env:TMP

try {
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'prepare-canonical-native-fixtures.ps1') -FlectBinary $FlectBinary -CaseRoot $caseRoot -ControllerRoot $controllerRoot -RunId dispatch-smoke | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'fixture preparation failed' }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    $hash = (Get-FileHash -LiteralPath $manifest.flect_path -Algorithm SHA256).Hash
    if ($hash -ne $manifest.flect_sha256) { throw 'manifest executable hash mismatch' }
    $env:TEMP = $manifest.agent_temp_root
    $env:TMP = $manifest.agent_temp_root

    $textRoot = Join-Path $root 'text'
    New-Item -ItemType Directory -Path $textRoot | Out-Null
    $objective = Join-Path $textRoot 'objective.txt'
    $after = Join-Path $textRoot 'after.txt'
    [IO.File]::WriteAllText($objective, 'smoke objective', [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($after, 'smoke after behavior', [Text.UTF8Encoding]::new($false))
    $v = @{}
    foreach ($entry in $manifest.verifier) { $v[$entry.name] = $entry.name }
    & $manifest.flect_path --json agent $v['verifier-begin'] --job $manifest.blind_job_id --model gpt-5.6-terra --model-selection explicit | Out-Null
    & $manifest.flect_path --json agent $v['verifier-set-objective'] --job $manifest.blind_job_id --text-file $objective | Out-Null
    & $manifest.flect_path --json agent $v['verifier-add-after'] --job $manifest.blind_job_id --text-file $after | Out-Null
    & $manifest.flect_path --json agent $v['verifier-add-scope'] --job $manifest.blind_job_id --file 'src/auth.rs' | Out-Null
    & $manifest.flect_path --json agent $v['verifier-set-confidence'] --job $manifest.blind_job_id 0.9 | Out-Null
    & $manifest.flect_path --json agent $v['verifier-submit'] --job $manifest.blind_job_id | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'verifier typed lifecycle failed' }

    Push-Location $case
    try {
        & $manifest.flect_path --json agent verifier-commit --job $manifest.blind_job_id | Out-Null
        $judge = & $manifest.flect_path --json agent prepare-reconciliation --blind-job $manifest.blind_job_id | ConvertFrom-Json
        $j = @{}
        foreach ($entry in $manifest.judge) { $j[$entry.name] = $entry.name }
        & $manifest.flect_path --json agent $j['judge-begin'] --job $judge.job_id --model gpt-5.6-terra --model-selection explicit | Out-Null
        & $manifest.flect_path --json agent $j['judge-set-alignment'] --job $judge.job_id same | Out-Null
        & $manifest.flect_path --json agent $j['judge-set-confidence'] --job $judge.job_id 0.9 | Out-Null
        & $manifest.flect_path --json agent $j['judge-submit'] --job $judge.job_id | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'judge typed lifecycle failed' }
    }
    finally { Pop-Location }
    [pscustomobject]@{ result = 'passed'; flect_path = $manifest.flect_path; sha256 = $manifest.flect_sha256; blind_job_id = $manifest.blind_job_id; judge_job_id = $judge.job_id; state_root = $manifest.agent_temp_root } | ConvertTo-Json -Compress
}
finally {
    $env:TEMP = $previousTemp
    $env:TMP = $previousTmp
}
