# Install the already-built executable after the running server is stopped.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$source = Join-Path $projectRoot 'target/release/rad-server.exe'
$destination = Join-Path $projectRoot 'dist/RadServer/rad-server.exe'
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
    throw 'Release build not found. Build rad-server first.'
}
$running = Get-Process -Name 'rad-server' -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $destination }
if ($running) { throw 'Stop RadServer before installing the update.' }
Copy-Item -LiteralPath $source -Destination $destination -Force
$manifestPath = Join-Path $projectRoot 'target/BUILD-MANIFEST.json'
if (Test-Path -LiteralPath $manifestPath) {
    $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
    $entry = $manifest.binaries | Where-Object service -eq 'RadServer' | Select-Object -First 1
    if ($entry) {
        $entry.sha256 = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
        $entry.size_bytes = (Get-Item -LiteralPath $destination).Length
        $manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding utf8
    }
}
Write-Host "RadServer updated: $destination"
