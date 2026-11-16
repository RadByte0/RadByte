param(
    [ValidateSet("Direct", "Network")]
    [string]$Profile = "Direct",
    [string]$WorkDir = (Split-Path -Parent $PSScriptRoot),
    [int]$ReadyTimeoutSec = 15
)

$ErrorActionPreference = "Stop"

function Get-BindEndpoint {
    param([string]$ConfigPath, [string]$DefaultHost, [int]$DefaultPort)
    if (Test-Path $ConfigPath) {
        $text = Get-Content -Raw -Encoding UTF8 $ConfigPath
        if ($text -match '(?m)^\s*server-port\s*=\s*(\d+)\s*$') {
            $p = [int]$Matches[1]
            $h = $DefaultHost
            if ($text -match '(?m)^\s*server-ip\s*=\s*([^\r\n]*)') { $h = $Matches[1].Trim() }
            if (-not $h -or $h -eq '0.0.0.0' -or $h -eq '::') { $h = '127.0.0.1' }
            return @{ Host = $h; Port = $p }
        }
        if ($text -match '(?m)^\s*bind[-_]addr\s*=\s*"?([^"\r\n#]+)') {
            $addr = $Matches[1].Trim()
            $idx = $addr.LastIndexOf(":")
            if ($idx -gt 0) {
                $h = $addr.Substring(0, $idx)
                if ($h -eq "0.0.0.0" -or $h -eq "::" -or $h -eq "[::]") { $h = "127.0.0.1" }
                $p = [int]$addr.Substring($idx + 1)
                return @{ Host = $h; Port = $p }
            }
        }
    }
    return @{ Host = $DefaultHost; Port = $DefaultPort }
}

function Wait-TcpReady {
    param([string]$TargetHost, [int]$Port, [int]$TimeoutSec, [System.Diagnostics.Process]$Proc)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        if ($Proc.HasExited) {
            throw "Process $($Proc.Id) exited early with code $($Proc.ExitCode) while waiting for ${TargetHost}:${Port}"
        }
        $client = New-Object System.Net.Sockets.TcpClient
        try {
            $ar = $client.BeginConnect($TargetHost, $Port, $null, $null)
            if ($ar.AsyncWaitHandle.WaitOne(200)) {
                $client.EndConnect($ar)
                return
            }
        }
        catch {
            # retry until deadline
        }
        finally {
            $client.Dispose()
        }
        Start-Sleep -Milliseconds 150
    }
    throw "Timed out waiting for TCP ${TargetHost}:${Port} after ${TimeoutSec}s"
}

$pidManifest = Join-Path $WorkDir ".radbyte-pids.json"
if (Test-Path $pidManifest) {
    $existing = Get-Content -Raw $pidManifest | ConvertFrom-Json
    foreach ($p in $existing.processes) {
        $alive = Get-Process -Id $p.pid -ErrorAction SilentlyContinue
        if ($alive -and $alive.Path -eq $p.exe_path) {
            throw "RadByte process $($p.name) (PID $($p.pid)) is already running. Run Stop-RadByte.ps1 first."
        }
    }
    Remove-Item -Force $pidManifest
}

$logsDir = Join-Path $WorkDir "logs"
New-Item -ItemType Directory -Force -Path $logsDir | Out-Null

$plan = if ($Profile -eq "Direct") {
    @(
        @{ Name = "RadServer"; Bin = "rad-server.exe"; Cfg = "rad.properties"; DefHost = "127.0.0.1"; DefPort = 25567 }
    )
} else {
    @(
        @{ Name = "RadLimbo";  Bin = "rad-limbo.exe";  Cfg = "rad.properties"; DefHost = "127.0.0.1"; DefPort = 25566 },
        @{ Name = "RadServer"; Bin = "rad-server.exe"; Cfg = "rad.properties"; DefHost = "127.0.0.1"; DefPort = 25567 },
        @{ Name = "RadProxy";  Bin = "rad-proxy.exe";  Cfg = "rad.properties"; DefHost = "127.0.0.1"; DefPort = 25565 }
    )
}

$started = @()
try {
    foreach ($svc in $plan) {
        $svcDir = Join-Path $WorkDir $svc.Name
        $exePath = Join-Path $svcDir $svc.Bin
        if (-not (Test-Path $exePath)) {
            throw "Missing binary: $exePath"
        }

        $stopFlag = Join-Path $svcDir ".radserver.stop"
        if (Test-Path $stopFlag) { Remove-Item -Force $stopFlag }

        $stdoutLog = Join-Path $logsDir ($svc.Name + ".stdout.log")
        $stderrLog = Join-Path $logsDir ($svc.Name + ".stderr.log")
        $cfgForEndpoint = Join-Path $svcDir $svc.Cfg
        if ($svc.Name -eq "RadServer" -and -not (Test-Path $cfgForEndpoint)) {
            $legacyCfg = Join-Path $svcDir "radserver.toml"
            if (Test-Path $legacyCfg) { $cfgForEndpoint = $legacyCfg }
        }
        $ep = Get-BindEndpoint -ConfigPath $cfgForEndpoint -DefaultHost $svc.DefHost -DefaultPort $svc.DefPort

        $startParams = @{}
        if ($svc.Name -eq "RadServer") { $startParams.ArgumentList = @('--nogui') }
        $proc = Start-Process @startParams `
            -FilePath $exePath `
            -WorkingDirectory $svcDir `
            -WindowStyle Hidden `
            -RedirectStandardOutput $stdoutLog `
            -RedirectStandardError $stderrLog `
            -PassThru

        $started += [PSCustomObject]@{
            name        = $svc.Name
            pid         = $proc.Id
            exe_path    = $exePath
            working_dir = $svcDir
            host        = $ep.Host
            port        = $ep.Port
            stdout_log  = $stdoutLog
            stderr_log  = $stderrLog
        }

        Wait-TcpReady -TargetHost $ep.Host -Port $ep.Port -TimeoutSec $ReadyTimeoutSec -Proc $proc
        Write-Host "[RadByte] $($svc.Name) ready on $($ep.Host):$($ep.Port) (PID $($proc.Id))" -ForegroundColor Green
    }

    $manifest = [PSCustomObject]@{
        profile        = $Profile
        started_at_utc = (Get-Date).ToUniversalTime().ToString("o")
        processes      = $started
    }
    $manifest | ConvertTo-Json -Depth 5 | Out-File -FilePath $pidManifest -Encoding utf8
    Write-Host "[RadByte] Started profile '$Profile' (manifest: $pidManifest)" -ForegroundColor Cyan
}
catch {
    Write-Warning "[RadByte] Startup failed: $($_.Exception.Message). Cleaning up started processes..."
    foreach ($s in $started) {
        $alive = Get-Process -Id $s.pid -ErrorAction SilentlyContinue
        if ($alive -and $alive.Path -eq $s.exe_path) {
            Stop-Process -Id $s.pid -Force -ErrorAction SilentlyContinue
        }
    }
    if (Test-Path $pidManifest) { Remove-Item -Force $pidManifest }
    throw
}
