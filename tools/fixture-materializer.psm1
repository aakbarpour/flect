Set-StrictMode -Version Latest

function Split-FixtureText {
    param([AllowEmptyString()][string]$Text)

    $newline = if ($Text.Contains("`r`n")) { "`r`n" } else { "`n" }
    $trailing = $Text.EndsWith($newline)
    $body = if ($trailing) { $Text.Substring(0, $Text.Length - $newline.Length) } else { $Text }
    $lines = if ($body.Length -eq 0) { @() } else { @($body -split [regex]::Escape($newline)) }
    [pscustomobject]@{ lines = $lines; newline = $newline; trailing = $trailing }
}

function Join-FixtureText {
    param([string[]]$Lines, [string]$Newline, [bool]$Trailing)

    $text = $Lines -join $Newline
    if ($Trailing) { $text += $Newline }
    return $text
}

function Get-FixtureHunks {
    param([string]$Patch, [string]$Path)

    $lines = @($Patch -split "`r?`n")
    if ($lines.Count -gt 0 -and $lines[-1] -eq '') {
        $lines = if ($lines.Count -eq 1) { @() } else { @($lines[0..($lines.Count - 2)]) }
    }
    $hunks = [System.Collections.Generic.List[object]]::new()
    $current = $null
    foreach ($line in $lines) {
        if ($line -match '^@@ -(?<old>\d+)(,(?<oldCount>\d+))? \+(?<new>\d+)(,(?<newCount>\d+))? @@') {
            if ($null -ne $current) {
                [string[]]$oldLines = $current.old.ToArray()
                [string[]]$newLines = $current.new.ToArray()
                [void]$hunks.Add([pscustomobject]@{ old = @($oldLines); new = @($newLines); old_declared = $current.old_declared; new_declared = $current.new_declared; count_metadata_valid = ($oldLines.Count -eq $current.old_declared -and $newLines.Count -eq $current.new_declared) })
            }
            $current = [ordered]@{
                old = [System.Collections.Generic.List[string]]::new()
                new = [System.Collections.Generic.List[string]]::new()
                old_declared = if ($Matches.ContainsKey('oldCount') -and $Matches.oldCount) { [int]$Matches.oldCount } else { 1 }
                new_declared = if ($Matches.ContainsKey('newCount') -and $Matches.newCount) { [int]$Matches.newCount } else { 1 }
                phase = $null
            }
            continue
        }
        if ($null -eq $current) { throw "fixture patch for $Path has content outside a hunk" }
        if ($line.StartsWith('\\')) { throw "fixture patch for $Path uses an unsupported no-newline marker" }
        if ($line.Length -eq 0) { throw "fixture patch for $Path has an unprefixed empty hunk line" }
        $text = $line.Substring(1)
        $prefix = $line.Substring(0, 1)
        if ($prefix.Equals('-', [System.StringComparison]::Ordinal)) { $current.old.Add($text); $current.phase = 'old'; continue }
        if ($prefix.Equals('+', [System.StringComparison]::Ordinal)) { $current.new.Add($text); $current.phase = 'new'; continue }
        if ($prefix.Equals(' ', [System.StringComparison]::Ordinal)) { $current.old.Add($text); $current.new.Add($text); continue }
        if ($current.phase -eq 'old') { $current.old.Add($line); continue }
        if ($current.phase -eq 'new') { $current.new.Add($line); continue }
        throw "fixture patch for $Path has an unprefixed line without a change side"
    }
    if ($null -ne $current) {
        [string[]]$oldLines = $current.old.ToArray()
        [string[]]$newLines = $current.new.ToArray()
        [void]$hunks.Add([pscustomobject]@{ old = @($oldLines); new = @($newLines); old_declared = $current.old_declared; new_declared = $current.new_declared; count_metadata_valid = ($oldLines.Count -eq $current.old_declared -and $newLines.Count -eq $current.new_declared) })
    }
    if ($hunks.Count -eq 0) { throw "fixture patch for $Path contains no hunks" }
    return $hunks.ToArray()
}

function Find-UnambiguousSequence {
    param([string[]]$Haystack, [string[]]$Needle, [string]$Path)

    if ($Needle.Count -eq 0) { throw "fixture hunk for $Path has no old/context anchor" }
    $matches = [System.Collections.Generic.List[int]]::new()
    for ($start = 0; $start -le $Haystack.Count - $Needle.Count; $start++) {
        $same = $true
        for ($offset = 0; $offset -lt $Needle.Count; $offset++) {
            if ($Haystack[$start + $offset] -cne $Needle[$offset]) { $same = $false; break }
        }
        if ($same) { $matches.Add($start) }
    }
    if ($matches.Count -eq 0) { throw "fixture hunk old/context sequence is absent from ${Path}: old=[$($Needle -join '|')], base=[$($Haystack -join '|')]" }
    if ($matches.Count -ne 1) { throw "fixture hunk old/context sequence is ambiguous in $Path" }
    return $matches[0]
}

function Get-StructuralContent {
    param([AllowEmptyString()][string]$BaseContent, [string]$Patch, [string]$Path)

    $base = Split-FixtureText $BaseContent
    $hunks = @(Get-FixtureHunks -Patch $Patch -Path $Path)
    if (@($base.lines).Count -eq 0 -and $hunks.Count -eq 1 -and @($hunks[0].old).Count -eq 0) {
        return [pscustomobject]@{
            content = Join-FixtureText -Lines $hunks[0].new.ToArray() -Newline "`n" -Trailing $false
            hunk_count_metadata_valid = $hunks[0].count_metadata_valid
        }
    }
    $matches = [System.Collections.Generic.List[object]]::new()
    foreach ($hunk in $hunks) {
        [string[]]$oldLines = @($hunk.old)
        [string[]]$newLines = @($hunk.new)
        $start = Find-UnambiguousSequence -Haystack $base.lines -Needle $oldLines -Path $Path
        [void]$matches.Add([pscustomobject]@{ start = $start; old = $oldLines; new = $newLines })
    }
    $ordered = @($matches | Sort-Object start -Descending)
    for ($index = 0; $index -lt $ordered.Count - 1; $index++) {
        $left = $ordered[$index]
        $right = $ordered[$index + 1]
        if ($right.start + $right.old.Count -gt $left.start) { throw "fixture hunks overlap in $Path" }
    }
    $result = [System.Collections.Generic.List[string]]::new()
    foreach ($line in $base.lines) { $result.Add($line) }
    foreach ($match in $ordered) {
        $result.RemoveRange($match.start, $match.old.Count)
        $result.InsertRange($match.start, [string[]]$match.new)
    }
    [pscustomobject]@{
        content = Join-FixtureText -Lines $result.ToArray() -Newline $base.newline -Trailing $base.trailing
        hunk_count_metadata_valid = @($hunks | Where-Object { -not $_.count_metadata_valid }).Count -eq 0
    }
}

function New-NativeCandidatePatch {
    param($Case)

    $lines = [System.Collections.Generic.List[string]]::new()
    foreach ($file in $Case.candidate_patch.files) {
        if ($file.binary) { throw "binary fixture patch requires surrogate materialization: $($file.path)" }
        $oldPath = if ($file.PSObject.Properties['old_path'] -and $file.old_path) { $file.old_path } else { $file.path }
        $lines.Add("diff --git a/$oldPath b/$($file.path)")
        switch ($file.status) {
            'modified' { $lines.Add("--- a/$oldPath"); $lines.Add("+++ b/$($file.path)") }
            'added' { $lines.Add('new file mode 100644'); $lines.Add('--- /dev/null'); $lines.Add("+++ b/$($file.path)") }
            'untracked' { $lines.Add('new file mode 100644'); $lines.Add('--- /dev/null'); $lines.Add("+++ b/$($file.path)") }
            'deleted' { $lines.Add('deleted file mode 100644'); $lines.Add("--- a/$oldPath"); $lines.Add('+++ /dev/null') }
            'renamed' { $lines.Add("--- a/$oldPath"); $lines.Add("+++ b/$($file.path)") }
            default { throw "unsupported fixture status $($file.status)" }
        }
        $lines.Add($file.patch)
    }
    return (($lines -join "`n") + "`n")
}

function Get-Sha256Hex {
    param([byte[]]$Bytes)

    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ($hasher.ComputeHash($Bytes) | ForEach-Object { $_.ToString('x2') }) -join ''
    }
    finally {
        $hasher.Dispose()
    }
}

function Set-PrivateBinaryAttribute {
    param([string]$CasePath, [string]$Path)

    if ($Path -match '(^|[\\/])\.\.?([\\/]|$)') { throw "unsafe binary fixture path: $Path" }
    $attributesPath = Join-Path $CasePath '.git\info\attributes'
    [System.IO.Directory]::CreateDirectory((Split-Path -Parent $attributesPath)) | Out-Null
    $line = "/$($Path.Replace('\\', '/')) binary"
    $existing = if (Test-Path -LiteralPath $attributesPath) { [System.IO.File]::ReadAllText($attributesPath) } else { '' }
    if ($existing.Length -ne 0 -and -not $existing.EndsWith("`n")) { $existing += "`n" }
    [System.IO.File]::WriteAllText($attributesPath, $existing + $line + "`n", [System.Text.UTF8Encoding]::new($false))
}

function New-BinarySurrogate {
    param([string]$CasePath, $File)

    if ($File.status -ne 'modified') { throw "binary surrogate supports only modified files: $($File.path)" }
    $target = Join-Path $CasePath $File.path
    if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { throw "binary surrogate base file is absent: $($File.path)" }

    $baseBytes = [System.IO.File]::ReadAllBytes($target)
    $beforeHash = Get-Sha256Hex $baseBytes
    # This seed intentionally hashes fixture representation, path, and base bytes; it never
    # interprets the textual placeholder patch as replacement file content.
    $record = [ordered]@{
        algorithm = 'flect-binary-surrogate-v1'
        path = $File.path
        status = $File.status
        binary = [bool]$File.binary
        candidate_patch_sha256 = Get-Sha256Hex ([System.Text.Encoding]::UTF8.GetBytes([string]$File.patch))
        base_sha256 = $beforeHash
    } | ConvertTo-Json -Compress
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $seed = $hasher.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($record))
    }
    finally {
        $hasher.Dispose()
    }
    $suffix = [byte[]](0x00, 0xf1, 0xec, 0x7a) + $seed
    $afterBytes = [byte[]]$baseBytes + $suffix
    [System.IO.File]::WriteAllBytes($target, $afterBytes)
    Set-PrivateBinaryAttribute $CasePath $File.path

    [pscustomobject]@{
        algorithm = 'flect-binary-surrogate-v1'
        record_sha256 = Get-Sha256Hex ([System.Text.Encoding]::UTF8.GetBytes($record))
        before_sha256 = $beforeHash
        after_sha256 = Get-Sha256Hex $afterBytes
    }
}

function Invoke-FixtureMaterialization {
    param([string]$CasePath, $Case, [string]$PatchPath)

    $binaryFiles = @($Case.candidate_patch.files | Where-Object { $_.binary })
    if ($binaryFiles.Count -gt 0) {
        if ($binaryFiles.Count -ne $Case.candidate_patch.files.Count) { throw 'binary surrogate cannot be combined with textual fixture files' }
        if ($binaryFiles.Count -ne 1) { throw 'binary surrogate requires exactly one binary fixture file' }
        [System.IO.File]::WriteAllText($PatchPath, [string]$binaryFiles[0].patch, [System.Text.UTF8Encoding]::new($false))
        $surrogate = New-BinarySurrogate $CasePath $binaryFiles[0]
        Assert-FixtureMaterialization $CasePath $Case -BinarySurrogate $surrogate
        return [pscustomobject]@{ mode = 'binary_surrogate'; hunk_count_metadata_valid = $null; binary_surrogate = $surrogate }
    }

    $native = New-NativeCandidatePatch $Case
    [System.IO.File]::WriteAllText($PatchPath, $native, [System.Text.UTF8Encoding]::new($false))
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & git -C $CasePath apply --check $PatchPath 2>$null
        $nativeExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($nativeExitCode -eq 0) {
        & git -C $CasePath apply --whitespace=nowarn $PatchPath
        if ($LASTEXITCODE -ne 0) { throw "exact git application unexpectedly failed for $($Case.id)" }
        try {
            Assert-FixtureMaterialization $CasePath $Case
            return [pscustomobject]@{ mode = 'git_exact'; hunk_count_metadata_valid = $true; binary_surrogate = $null }
        }
        catch {
            & git -C $CasePath reset --hard HEAD | Out-Null
            if ($LASTEXITCODE -ne 0) { throw "could not reset invalid native fixture application for $($Case.id)" }
        }
    }

    $countMetadataValid = $true
    foreach ($file in $Case.candidate_patch.files) {
        $oldPath = if ($file.PSObject.Properties['old_path'] -and $file.old_path) { $file.old_path } else { $file.path }
        $oldFullPath = Join-Path $CasePath $oldPath
        $newFullPath = Join-Path $CasePath $file.path
        switch ($file.status) {
            'modified' {
                if (-not (Test-Path -LiteralPath $oldFullPath -PathType Leaf)) { throw "base file is absent: $oldPath" }
                $materialized = Get-StructuralContent -BaseContent ([System.IO.File]::ReadAllText($oldFullPath)) -Patch $file.patch -Path $file.path
                [System.IO.File]::WriteAllText($newFullPath, $materialized.content, [System.Text.UTF8Encoding]::new($false))
                $countMetadataValid = $countMetadataValid -and $materialized.hunk_count_metadata_valid
            }
            'deleted' {
                if (-not (Test-Path -LiteralPath $oldFullPath -PathType Leaf)) { throw "base file is absent: $oldPath" }
                $materialized = Get-StructuralContent -BaseContent ([System.IO.File]::ReadAllText($oldFullPath)) -Patch $file.patch -Path $oldPath
                if ($materialized.content.Length -ne 0) { throw "deleted fixture leaves content in $oldPath" }
                Remove-Item -LiteralPath $oldFullPath
                $countMetadataValid = $countMetadataValid -and $materialized.hunk_count_metadata_valid
            }
            'added' {
                if (Test-Path -LiteralPath $newFullPath) { throw "added fixture path already exists: $($file.path)" }
                $materialized = Get-StructuralContent -BaseContent '' -Patch $file.patch -Path $file.path
                [System.IO.Directory]::CreateDirectory((Split-Path -Parent $newFullPath)) | Out-Null
                [System.IO.File]::WriteAllText($newFullPath, $materialized.content, [System.Text.UTF8Encoding]::new($false))
                & git -C $CasePath add -- $file.path
                if ($LASTEXITCODE -ne 0) { throw "could not stage added fixture path $($file.path)" }
                $countMetadataValid = $countMetadataValid -and $materialized.hunk_count_metadata_valid
            }
            'untracked' {
                if (Test-Path -LiteralPath $newFullPath) { throw "untracked fixture path already exists: $($file.path)" }
                $materialized = Get-StructuralContent -BaseContent '' -Patch $file.patch -Path $file.path
                [System.IO.Directory]::CreateDirectory((Split-Path -Parent $newFullPath)) | Out-Null
                [System.IO.File]::WriteAllText($newFullPath, $materialized.content, [System.Text.UTF8Encoding]::new($false))
                $countMetadataValid = $countMetadataValid -and $materialized.hunk_count_metadata_valid
            }
            'renamed' {
                if (-not ($file.PSObject.Properties['old_path'] -and $file.old_path)) { throw "renamed fixture lacks old_path: $($file.path)" }
                if (-not (Test-Path -LiteralPath $oldFullPath -PathType Leaf)) { throw "base file is absent: $oldPath" }
                [System.IO.Directory]::CreateDirectory((Split-Path -Parent $newFullPath)) | Out-Null
                Move-Item -LiteralPath $oldFullPath -Destination $newFullPath
                $materialized = Get-StructuralContent -BaseContent ([System.IO.File]::ReadAllText($newFullPath)) -Patch $file.patch -Path $file.path
                [System.IO.File]::WriteAllText($newFullPath, $materialized.content, [System.Text.UTF8Encoding]::new($false))
                & git -C $CasePath add -A -- $oldPath $file.path
                if ($LASTEXITCODE -ne 0) { throw "could not stage renamed fixture path $($file.path)" }
                $countMetadataValid = $countMetadataValid -and $materialized.hunk_count_metadata_valid
            }
            default { throw "unsupported fixture status $($file.status)" }
        }
    }
    return [pscustomobject]@{ mode = 'fixture_structural'; hunk_count_metadata_valid = $countMetadataValid; binary_surrogate = $null }
}

function Assert-FixtureMaterialization {
    param([string]$CasePath, $Case, $BinarySurrogate)

    $expectedPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($file in $Case.candidate_patch.files) {
        $oldPath = if ($file.PSObject.Properties['old_path'] -and $file.old_path) { $file.old_path } else { $file.path }
        if ($file.binary) {
            if ($null -eq $BinarySurrogate) { throw "binary fixture requires surrogate metadata: $($file.path)" }
            if ($file.status -ne 'modified') { throw "binary surrogate status is unsupported: $($file.status)" }
            $target = Join-Path $CasePath $file.path
            if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { throw "binary surrogate file is absent: $($file.path)" }
            $actualHash = Get-Sha256Hex ([System.IO.File]::ReadAllBytes($target))
            if ($actualHash -cne $BinarySurrogate.after_sha256) { throw "binary surrogate bytes differ from recorded hash: $($file.path)" }
            $attribute = & git -C $CasePath check-attr binary -- $file.path
            if ($LASTEXITCODE -ne 0 -or $attribute -notmatch ': binary: set$') { throw "Git did not classify surrogate as binary: $($file.path)" }
            $null = $expectedPaths.Add($file.path)
            continue
        }
        $baseFile = @($Case.base_files | Where-Object { $_.path -ceq $oldPath })
        $baseContent = if ($baseFile.Count -eq 1) { $baseFile[0].content } else { '' }
        $expected = Get-StructuralContent -BaseContent $baseContent -Patch $file.patch -Path $file.path
        $target = Join-Path $CasePath $file.path
        switch ($file.status) {
            'modified' {
                if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { throw "materialized file is absent: $($file.path)" }
                if ([System.IO.File]::ReadAllText($target) -cne $expected.content) { throw "materialized content differs from fixture hunk body: $($file.path)" }
                $null = $expectedPaths.Add($file.path)
            }
            'deleted' {
                if (Test-Path -LiteralPath (Join-Path $CasePath $oldPath)) { throw "deleted fixture path remains: $oldPath" }
                $null = $expectedPaths.Add($oldPath)
            }
            'added' { if ([System.IO.File]::ReadAllText($target) -cne $expected.content) { throw "materialized content differs from fixture hunk body: $($file.path)" }; $null = $expectedPaths.Add($file.path) }
            'untracked' { if ([System.IO.File]::ReadAllText($target) -cne $expected.content) { throw "materialized content differs from fixture hunk body: $($file.path)" }; $null = $expectedPaths.Add($file.path) }
            'renamed' {
                if (Test-Path -LiteralPath (Join-Path $CasePath $oldPath)) { throw "renamed fixture source remains: $oldPath" }
                if ([System.IO.File]::ReadAllText($target) -cne $expected.content) { throw "materialized content differs from fixture hunk body: $($file.path)" }
                $null = $expectedPaths.Add($oldPath); $null = $expectedPaths.Add($file.path)
            }
            default { throw "unsupported fixture status $($file.status)" }
        }
    }
    $changed = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($path in @(& git -C $CasePath diff --name-only HEAD)) { if ($path) { $null = $changed.Add($path) } }
    foreach ($path in @(& git -C $CasePath ls-files --others --exclude-standard)) { if ($path) { $null = $changed.Add($path) } }
    if (-not $changed.SetEquals($expectedPaths)) {
        throw "materialization changed unexpected paths: expected [$($expectedPaths -join ', ')], got [$($changed -join ', ')]"
    }
}

Export-ModuleMember -Function Get-FixtureHunks, Get-StructuralContent, New-NativeCandidatePatch, Invoke-FixtureMaterialization, Assert-FixtureMaterialization
