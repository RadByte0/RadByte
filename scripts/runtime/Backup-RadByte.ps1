param(
    [string]$WorldDir = (Join-Path (Split-Path -Parent $PSScriptRoot) "RadServer\world"),
    [string]$BackupDir = (Join-Path (Split-Path -Parent $PSScriptRoot) "backups"),
    [string]$WorkDir = (Split-Path -Parent $PSScriptRoot),
    [string]$RestoreTo = "",
    [switch]$AllowLive
)

$ErrorActionPreference = "Stop"

$pidManifest = Join-Path $WorkDir ".radbyte-pids.json"
if (-not $AllowLive -and (Test-Path $pidManifest)) {
    $manifest = Get-Content -Raw $pidManifest | ConvertFrom-Json
    foreach ($p in $manifest.processes) {
        if ($p.name -eq "RadServer") {
            $alive = Get-Process -Id $p.pid -ErrorAction SilentlyContinue
            if ($alive -and $alive.Path -eq $p.exe_path) {
                throw "RadServer (PID $($p.pid)) is currently running. Stop it with Stop-RadByte.ps1 before backing up (or pass -AllowLive)."
            }
        }
    }
}

if (-not (Test-Path $WorldDir)) {
    throw "World directory does not exist: $WorldDir"
}

$stamp = (Get-Date).ToUniversalTime().ToString("yyyyMMdd-HHmmss")
$target = Join-Path $BackupDir ("world-" + $stamp)
New-Item -ItemType Directory -Force -Path $target | Out-Null

$worldTarget = Join-Path $target "world"
Copy-Item -Recurse -Force $WorldDir $worldTarget

$files = Get-ChildItem -Path $worldTarget -Recurse -File | Sort-Object FullName
$entries = @()
foreach ($f in $files) {
    $rel = $f.FullName.Substring($worldTarget.Length).TrimStart('\', '/')
    $hash = (Get-FileHash -Algorithm SHA256 -Path $f.FullName).Hash.ToLowerInvariant()
    $entries += [PSCustomObject]@{
        relative_path = $rel
        sha256        = $hash
        size_bytes    = $f.Length
    }
}

$checksumManifest = [PSCustomObject]@{
    created_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    source_world   = $WorldDir
    files          = $entries
}
$manifestPath = Join-Path $target "manifest.sha256.json"
$checksumManifest | ConvertTo-Json -Depth 5 | Out-File -FilePath $manifestPath -Encoding utf8
Write-Host "[RadByte] Backup created: $target ($($entries.Count) files)" -ForegroundColor Green

if ($RestoreTo) {
    if (Test-Path $RestoreTo) { Remove-Item -Recurse -Force $RestoreTo }
    Copy-Item -Recurse -Force $worldTarget $RestoreTo
    foreach ($entry in $entries) {
        $restoredFile = Join-Path $RestoreTo $entry.relative_path
        if (-not (Test-Path $restoredFile)) {
            throw "Restore verification failed: missing $restoredFile"
        }
        $actual = (Get-FileHash -Algorithm SHA256 -Path $restoredFile).Hash.ToLowerInvariant()
        if ($actual -ne $entry.sha256) {
            throw "Restore verification failed: SHA-256 mismatch on $($entry.relative_path)"
        }
    }
    Write-Host "[RadByte] Restore verification passed -> $RestoreTo ($($entries.Count) files verified)" -ForegroundColor Green
}

return $target
