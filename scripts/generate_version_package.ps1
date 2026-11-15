param(
    [string]$GameVersion = "26.3",
    [int]$ProtocolVersion = 777,
    [int]$WorldDataVersion = 4790,
    [string]$LibName = "rad_version_26_3",
    [string]$DataDir = "",
    [string]$SourceLabel = "external-data-package (inspect provenance)",
    [string]$OutDir = "",
    [string]$Profile = "release",
    [string]$LibraryPath = "",
    [string]$BackupDirectory = "",
    [switch]$BackupExisting
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $root ("versions\" + $GameVersion)
}
if ([string]::IsNullOrWhiteSpace($DataDir)) {
    throw "Missing -DataDir: provide an explicit path to a compatible version data package (blocks.json, items.json, entities.json, menus.json)."
}
$srcDataDir = $DataDir

if (-not (Test-Path -LiteralPath $srcDataDir)) {
    throw "Missing source data directory: $srcDataDir"
}

$parentDir = Split-Path -Parent $OutDir
New-Item -ItemType Directory -Force -Path $parentDir | Out-Null

# Preserve an existing selection; package generation must never reset it.
$versionSelection = Join-Path $parentDir 'version.toml'
if (-not (Test-Path -LiteralPath $versionSelection)) {
    $selectionStream = [System.IO.File]::Open($versionSelection, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::Read)
    try {
        $selectionBytes = [System.Text.UTF8Encoding]::new($false).GetBytes("# Select an installed and supported Minecraft version.`nversion = `"$GameVersion`"`n")
        $selectionStream.Write($selectionBytes, 0, $selectionBytes.Length)
    } finally { $selectionStream.Dispose() }
}

$stageDir = Join-Path $parentDir ($GameVersion + ".staging." + $PID)
if (Test-Path -LiteralPath $stageDir) {
    Remove-Item -Recurse -Force -LiteralPath $stageDir
}

$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

try {
    $stageDataDir = Join-Path $stageDir "data"
    New-Item -ItemType Directory -Force -Path $stageDataDir | Out-Null

    $datasets = @(
        @{ Id = "items";    File = "items.json";    Prop = "entries" },
        @{ Id = "blocks";   File = "blocks.json";   Prop = "blocks" },
        @{ Id = "entities"; File = "entities.json"; Prop = "entries" },
        @{ Id = "menus";    File = "menus.json";    Prop = "entries" }
    )

    $manifestFiles = @()
    foreach ($ds in $datasets) {
        $srcPath = Join-Path $srcDataDir $ds.File
        $dstPath = Join-Path $stageDataDir $ds.File
        if (-not (Test-Path -LiteralPath $srcPath)) {
            throw "Missing dataset file: $srcPath"
        }
        Copy-Item -LiteralPath $srcPath -Destination $dstPath -Force

        $parsed = Get-Content -Raw -Encoding UTF8 -LiteralPath $dstPath | ConvertFrom-Json
        $count = ($parsed.($ds.Prop)).Count
        if ($count -le 0) {
            throw "Dataset $($ds.File) has zero entries"
        }

        $bytes = (Get-Item -LiteralPath $dstPath).Length
        $sha = (Get-FileHash -Algorithm SHA256 -LiteralPath $dstPath).Hash.ToLowerInvariant()
        $manifestFiles += [PSCustomObject]@{
            id          = $ds.Id
            path        = "data/$($ds.File)"
            size_bytes  = [uint64]$bytes
            sha256      = $sha
            entry_count = [int]$count
        }
    }

    $optionalTagsSrc = Join-Path $srcDataDir "tags.json"
    if (Test-Path -LiteralPath $optionalTagsSrc) {
        $optionalTagsDst = Join-Path $stageDataDir "tags.json"
        Copy-Item -LiteralPath $optionalTagsSrc -Destination $optionalTagsDst -Force
        $parsedTags = Get-Content -Raw -Encoding UTF8 -LiteralPath $optionalTagsDst | ConvertFrom-Json
        $tagCount = 0
        if ($null -ne $parsedTags.block_tags) { $tagCount += $parsedTags.block_tags.Count }
        if ($null -ne $parsedTags.item_tags) { $tagCount += $parsedTags.item_tags.Count }
        if ($null -ne $parsedTags.core_registry_tags) {
            foreach ($regSet in $parsedTags.core_registry_tags) {
                if ($null -ne $regSet.tags) { $tagCount += $regSet.tags.Count }
            }
        }
        $bytes = (Get-Item -LiteralPath $optionalTagsDst).Length
        $sha = (Get-FileHash -Algorithm SHA256 -LiteralPath $optionalTagsDst).Hash.ToLowerInvariant()
        $manifestFiles += [PSCustomObject]@{
            id          = "tags"
            path        = "data/tags.json"
            size_bytes  = [uint64]$bytes
            sha256      = $sha
            entry_count = [int]$tagCount
        }
    }

    if ([string]::IsNullOrWhiteSpace($LibraryPath)) {
        if ($IsLinux) {
            $libFileName = "lib$LibName.so"
        } elseif ($IsMacOS) {
            $libFileName = "lib$LibName.dylib"
        } else {
            $libFileName = "$LibName.dll"
        }
        $LibraryPath = Join-Path $root ("target\$Profile\$libFileName")
    } else {
        $libFileName = Split-Path -Leaf $LibraryPath
    }
    if (-not (Test-Path -LiteralPath $LibraryPath)) {
        throw "Missing compiled version library: $LibraryPath (build the version crate first)"
    }
    $dstLibPath = Join-Path $stageDir $libFileName
    Copy-Item -LiteralPath $LibraryPath -Destination $dstLibPath -Force
    $libBytes = (Get-Item -LiteralPath $dstLibPath).Length
    $libSha = (Get-FileHash -Algorithm SHA256 -LiteralPath $dstLibPath).Hash.ToLowerInvariant()
    $targetOs = if ($IsLinux) { "linux" } elseif ($IsMacOS) { "macos" } else { "windows" }
    $targetArch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()) {
        "x64"   { "x86_64" }
        "arm64" { "aarch64" }
        default { [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant() }
    }

    $manifest = [PSCustomObject]@{
        schema_version         = 1
        game_version           = $GameVersion
        protocol_version       = $ProtocolVersion
        package_format_version = 1
        world_data_version     = $WorldDataVersion
        source                 = $SourceLabel
        generated_utc          = (Get-Date).ToUniversalTime().ToString("o")
        library                = [PSCustomObject]@{
            path        = $libFileName
            abi_version = 1
            target_os   = $targetOs
            target_arch = $targetArch
            size_bytes  = [uint64]$libBytes
            sha256      = $libSha
        }
        files                  = $manifestFiles
    }

    $manifestJson = $manifest | ConvertTo-Json -Depth 5
    [System.IO.File]::WriteAllText((Join-Path $stageDir "version.json"), $manifestJson, $utf8NoBom)

    if (Test-Path -LiteralPath $OutDir) {
        if ($BackupExisting) {
            $backupPath = "$OutDir.bak." + (Get-Date -Format "yyyyMMdd-HHmmss")
            if ($BackupDirectory) {
                New-Item -ItemType Directory -Force -Path $BackupDirectory | Out-Null
                $backupPath = Join-Path $BackupDirectory ((Split-Path -Leaf $OutDir) + '.bak.' + [guid]::NewGuid().ToString('N'))
            }
            Move-Item -LiteralPath $OutDir -Destination $backupPath
        } else {
            Remove-Item -Recurse -Force -LiteralPath $OutDir
        }
    }
    Move-Item -LiteralPath $stageDir -Destination $OutDir
    Write-Host "[RadByte] Generated version package $GameVersion -> $OutDir" -ForegroundColor Green
}
finally {
    if (Test-Path -LiteralPath $stageDir) {
        Remove-Item -Recurse -Force -LiteralPath $stageDir -ErrorAction SilentlyContinue
    }
}
