param(
    [string]$WorkDir = (Split-Path -Parent $PSScriptRoot),
    [int]$TimeoutSec = 15
)

$ErrorActionPreference = "Stop"

$pidManifest = Join-Path $WorkDir ".radbyte-pids.json"
if (-not (Test-Path $pidManifest)) {
    Write-Host "[RadByte] No PID manifest at $pidManifest; nothing to stop." -ForegroundColor Yellow
    exit 0
}

$manifest = Get-Content -Raw $pidManifest | ConvertFrom-Json
$procs = @($manifest.processes)
[Array]::Reverse($procs)

# Signal RadServer graceful shutdown first so it runs save_all() before exit.
foreach ($p in $procs) {
    if ($p.name -eq "RadServer" -and $p.working_dir) {
        $stopFile = Join-Path $p.working_dir ".radserver.stop"
        [System.IO.File]::WriteAllText($stopFile, "stop`n", [System.Text.UTF8Encoding]::new($false))
    }
}

foreach ($p in $procs) {
    $alive = Get-Process -Id $p.pid -ErrorAction SilentlyContinue
    if (-not $alive) { continue }
    if ($alive.Path -and $p.exe_path -and ($alive.Path -ne $p.exe_path)) {
        Write-Warning "[RadByte] PID $($p.pid) path '$($alive.Path)' does not match '$($p.exe_path)'; skipping."
        continue
    }

    if ($p.name -eq "RadServer") {
        $deadline = (Get-Date).AddSeconds($TimeoutSec)
        while (-not $alive.HasExited -and (Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 150
            $alive.Refresh()
        }
        if (-not $alive.HasExited) {
            Write-Warning "[RadByte] RadServer (PID $($p.pid)) did not exit within ${TimeoutSec}s; stopping by verified PID."
            Stop-Process -Id $p.pid -Force -ErrorAction SilentlyContinue
            $alive.WaitForExit(3000) | Out-Null
        }
        $stopFile = Join-Path $p.working_dir ".radserver.stop"
        if (Test-Path $stopFile) { Remove-Item -Force $stopFile -ErrorAction SilentlyContinue }
    }
    else {
        Stop-Process -Id $p.pid -ErrorAction SilentlyContinue
        if (-not $alive.WaitForExit(5000)) {
            Stop-Process -Id $p.pid -Force -ErrorAction SilentlyContinue
            $alive.WaitForExit(3000) | Out-Null
        }
    }
    Write-Host "[RadByte] Stopped $($p.name) (PID $($p.pid))" -ForegroundColor Green
}

Remove-Item -Force $pidManifest -ErrorAction SilentlyContinue
