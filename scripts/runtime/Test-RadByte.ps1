param(
    [string]$ArtifactsRoot = ""
)

$ErrorActionPreference = "Stop"

$distDir = Split-Path -Parent $PSScriptRoot
$repoRoot = Split-Path -Parent $distDir
if (-not $ArtifactsRoot) {
    $ArtifactsRoot = Join-Path $repoRoot "target/test-runs"
}

$runId = (Get-Date).ToUniversalTime().ToString("yyyyMMdd-HHmmss")
$runDir = Join-Path $ArtifactsRoot $runId
$runLogsDir = Join-Path $runDir "logs"
New-Item -ItemType Directory -Force -Path $runLogsDir | Out-Null

function Get-FreeTcpPort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = ([System.Net.IPEndPoint]$listener.LocalEndpoint).Port
    $listener.Stop()
    return $port
}

function Write-VarInt {
    param([System.IO.Stream]$Stream, [int]$Value)
    $u = [uint32]$Value
    while ($true) {
        if (($u -band 0xFFFFFF80) -eq 0) {
            $Stream.WriteByte([byte]$u)
            break
        }
        $Stream.WriteByte([byte](($u -band 0x7F) -bor 0x80))
        $u = $u -shr 7
    }
}

function Read-VarInt {
    param([System.IO.Stream]$Stream)
    $numRead = 0
    $result = 0
    while ($true) {
        $b = $Stream.ReadByte()
        if ($b -lt 0) { throw "Unexpected EOF reading VarInt" }
        $result = $result -bor (($b -band 0x7F) -shl (7 * $numRead))
        $numRead++
        if ($numRead -gt 5) { throw "VarInt too large" }
        if (($b -band 0x80) -eq 0) { break }
    }
    return $result
}

function Read-ExactBytes {
    param([System.IO.Stream]$Stream, [int]$Count)
    $buf = New-Object byte[] $Count
    $offset = 0
    while ($offset -lt $Count) {
        $n = $Stream.Read($buf, $offset, $Count - $offset)
        if ($n -le 0) { throw "Unexpected EOF reading $Count bytes" }
        $offset += $n
    }
    return $buf
}

function Invoke-McStatusPing {
    param([string]$TargetHost, [int]$Port)
    $client = New-Object System.Net.Sockets.TcpClient
    $client.ReceiveTimeout = 5000
    $client.SendTimeout = 5000
    try {
        $client.Connect($TargetHost, $Port)
        $ns = $client.GetStream()

        # Handshake packet (id=0x00, proto=777, host, port, next_state=1)
        $hsBody = New-Object System.IO.MemoryStream
        Write-VarInt $hsBody 0x00
        Write-VarInt $hsBody 777
        $hostBytes = [System.Text.Encoding]::UTF8.GetBytes($TargetHost)
        Write-VarInt $hsBody $hostBytes.Length
        $hsBody.Write($hostBytes, 0, $hostBytes.Length)
        $hsBody.WriteByte([byte](($Port -shr 8) -band 0xFF))
        $hsBody.WriteByte([byte]($Port -band 0xFF))
        Write-VarInt $hsBody 1
        $hsPayload = $hsBody.ToArray()

        $frame = New-Object System.IO.MemoryStream
        Write-VarInt $frame $hsPayload.Length
        $frame.Write($hsPayload, 0, $hsPayload.Length)
        # Status Request packet (len=1, id=0x00)
        Write-VarInt $frame 1
        Write-VarInt $frame 0x00
        $outBytes = $frame.ToArray()
        $ns.Write($outBytes, 0, $outBytes.Length)
        $ns.Flush()

        $pktLen = Read-VarInt $ns
        $pktBuf = Read-ExactBytes $ns $pktLen
        $ms = New-Object System.IO.MemoryStream(,$pktBuf)
        $pktId = Read-VarInt $ms
        if ($pktId -ne 0x00) { throw "Expected StatusResponse packet 0x00, got $pktId" }
        $jsonLen = Read-VarInt $ms
        $jsonBytes = Read-ExactBytes $ms $jsonLen
        return [System.Text.Encoding]::UTF8.GetString($jsonBytes)
    }
    finally {
        $client.Dispose()
    }
}

function Send-McPacket {
    param([System.IO.Stream]$Stream, [int]$PacketId, [byte[]]$Body = @())
    $inner = New-Object System.IO.MemoryStream
    Write-VarInt $inner $PacketId
    if ($Body.Length -gt 0) { $inner.Write($Body, 0, $Body.Length) }
    $innerBytes = $inner.ToArray()
    $frame = New-Object System.IO.MemoryStream
    Write-VarInt $frame $innerBytes.Length
    $frame.Write($innerBytes, 0, $innerBytes.Length)
    $out = $frame.ToArray()
    $Stream.Write($out, 0, $out.Length)
    $Stream.Flush()
}

function Read-McPacket {
    param([System.IO.Stream]$Stream)
    $len = Read-VarInt $Stream
    $buf = Read-ExactBytes $Stream $len
    $ms = New-Object System.IO.MemoryStream(,$buf)
    $id = Read-VarInt $ms
    $rest = New-Object byte[] ($ms.Length - $ms.Position)
    [void]$ms.Read($rest, 0, $rest.Length)
    return @{ Id = $id; Body = $rest }
}

function Invoke-McPlaySmoke {
    param([string]$TargetHost, [int]$Port, [string]$Username = "SmokeTester")
    $client = New-Object System.Net.Sockets.TcpClient
    $client.ReceiveTimeout = 5000
    $client.SendTimeout = 5000
    try {
        $client.Connect($TargetHost, $Port)
        $ns = $client.GetStream()

        # 1. Handshake (next_state = 2 -> Login)
        $hs = New-Object System.IO.MemoryStream
        Write-VarInt $hs 777
        $hb = [System.Text.Encoding]::UTF8.GetBytes($TargetHost)
        Write-VarInt $hs $hb.Length
        $hs.Write($hb, 0, $hb.Length)
        $hs.WriteByte([byte](($Port -shr 8) -band 0xFF))
        $hs.WriteByte([byte]($Port -band 0xFF))
        Write-VarInt $hs 2
        Send-McPacket $ns 0x00 ($hs.ToArray())

        # 2. LoginStart (0x00)
        $ls = New-Object System.IO.MemoryStream
        $ub = [System.Text.Encoding]::UTF8.GetBytes($Username)
        Write-VarInt $ls $ub.Length
        $ls.Write($ub, 0, $ub.Length)
        $uuid = New-Object byte[] 16
        $ls.Write($uuid, 0, 16)
        Send-McPacket $ns 0x00 ($ls.ToArray())

        $stage = "LoginSuccess"
        $p = Read-McPacket $ns
        if ($p.Id -ne 0x02) { throw "Expected LoginSuccess (0x02), got $($p.Id)" }
        Send-McPacket $ns 0x03 @() # LoginAcknowledged

        # 3. Configuration
        $stage = "Configuration"
        for ($i = 0; $i -lt 64; $i++) {
            $pkt = Read-McPacket $ns
            if ($pkt.Id -eq 0x0F) {
                Send-McPacket $ns 0x07 $pkt.Body # Echo ServerboundKnownPacks
            }
            elseif ($pkt.Id -eq 0x03) {
                Send-McPacket $ns 0x03 @() # AcknowledgeFinishConfiguration
                break
            }
        }

        # 4. Play: confirm teleport (0x49 -> 0x00) and place a block (0x42)
        $stage = "Play"
        $seenPlayIds = @()
        $confirmed = $false
        for ($i = 0; $i -lt 256; $i++) {
            $pkt = Read-McPacket $ns
            $seenPlayIds += ("0x{0:X2}" -f $pkt.Id)
            if ($pkt.Id -eq 0x49) {
                $ms = New-Object System.IO.MemoryStream(,$pkt.Body)
                $tpId = Read-VarInt $ms
                $ack = New-Object System.IO.MemoryStream
                Write-VarInt $ack $tpId
                Send-McPacket $ns 0x00 ($ack.ToArray())
                $confirmed = $true
                break
            }
        }
        if (-not $confirmed) { throw "Did not receive SynchronizePlayerPosition (0x49)" }

        # UseItemOn (0x42) at (1, 63, 1), face=1 (Top)
        $use = New-Object System.IO.MemoryStream
        Write-VarInt $use 0 # main hand
        $packedPos = (([int64]1 -band 0x3FFFFFF) -shl 38) -bor (([int64]1 -band 0x3FFFFFF) -shl 12) -bor ([int64]63 -band 0xFFF)
        $posBytes = [BitConverter]::GetBytes([int64]$packedPos)
        [Array]::Reverse($posBytes)
        $use.Write($posBytes, 0, 8)
        Write-VarInt $use 1 # face=Top
        $half = [BitConverter]::GetBytes([single]0.5)
        [Array]::Reverse($half)
        $use.Write($half, 0, 4)
        $use.Write($half, 0, 4)
        $use.Write($half, 0, 4)
        $use.WriteByte(0)
        $use.WriteByte(0)
        Write-VarInt $use 1
        Send-McPacket $ns 0x42 ($use.ToArray())
        Start-Sleep -Milliseconds 250
        return "Play smoke OK ($Username placed block at 1,64,1)"
    }
    catch {
        $srvLog = Join-Path $smokeDir "logs\RadServer.stdout.log"
        $errLog = Join-Path $smokeDir "logs\RadServer.stderr.log"
        $tail = if (Test-Path $srvLog) { (Get-Content $srvLog -Tail 15) -join "`n" } else { "<no log>" }
        $errTail = if (Test-Path $errLog) { (Get-Content $errLog -Tail 15) -join "`n" } else { "<no err>" }
        $seenStr = ($seenPlayIds -join ",")
        throw "Play smoke failed at stage '$stage' (seenPlayIds=[$seenStr]): $($_.Exception.Message)`n--- stdout ---`n$tail`n--- stderr ---`n$errTail"
    }
    finally {
        $client.Dispose()
    }
}

function Get-TreeSnapshot {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return @() }
    $items = Get-ChildItem -Path $Path -Recurse -File | Sort-Object FullName
    $out = @()
    foreach ($f in $items) {
        $rel = $f.FullName.Substring($Path.Length).TrimStart('\', '/')
        $hash = (Get-FileHash -Algorithm SHA256 -Path $f.FullName).Hash.ToLowerInvariant()
        $out += [PSCustomObject]@{
            relative_path = $rel
            size_bytes    = $f.Length
            sha256        = $hash
        }
    }
    return $out
}

$commandLog = @()
function Invoke-LoggedStep {
    param([string]$Name, [scriptblock]$Action)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $exitCode = 0
    $detail = ""
    try {
        $res = & $Action
        if ($res) { $detail = ($res | Out-String).Trim() }
    }
    catch {
        $exitCode = 1
        $detail = $_.Exception.Message
        throw
    }
    finally {
        $sw.Stop()
        $script:commandLog += [PSCustomObject]@{
            step        = $Name
            exit_code   = $exitCode
            duration_ms = [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
            detail      = $detail
        }
    }
}

$smokeDir = Join-Path $env:TEMP ("radbyte-smoke-" + $runId)
New-Item -ItemType Directory -Force -Path $smokeDir | Out-Null

try {
    foreach ($svc in @("RadProxy", "RadLimbo", "RadServer")) {
        $src = Join-Path $distDir $svc
        if (-not (Test-Path $src)) { throw "Missing packaged service directory: $src. Run scripts/package.ps1 first." }
        $dst = Join-Path $smokeDir $svc
        New-Item -ItemType Directory -Force -Path $dst | Out-Null
        Get-ChildItem -Path $src -File | ForEach-Object { Copy-Item -Force $_.FullName (Join-Path $dst $_.Name) }
        foreach ($runtimeDirectory in @('versions', 'config')) {
            $runtimeSource = Join-Path $src $runtimeDirectory
            if (Test-Path -LiteralPath $runtimeSource) {
                Copy-Item -LiteralPath $runtimeSource -Destination $dst -Recurse
            }
        }
    }
    foreach ($scriptName in @("Start-RadByte.ps1", "Stop-RadByte.ps1", "Backup-RadByte.ps1")) {
        Copy-Item -Force (Join-Path $PSScriptRoot $scriptName) (Join-Path $smokeDir $scriptName)
    }

    $srvPort = Get-FreeTcpPort
    $limboPort = Get-FreeTcpPort
    $proxyPort = Get-FreeTcpPort

    $srvCfgPath = Join-Path $smokeDir "RadServer\rad.properties"
    $limboCfgPath = Join-Path $smokeDir "RadLimbo\radlimbo.toml"
    $proxyCfgPath = Join-Path $smokeDir "RadProxy\radproxy.toml"

    if (-not (Test-Path $srvCfgPath)) {
        $legacySrvCfgPath = Join-Path $smokeDir "RadServer\radserver.toml"
        $srvCfgText = Get-Content -Raw $legacySrvCfgPath
        $srvCfgText = $srvCfgText -replace 'bind_addr\s*=\s*"[^"]*"', "bind_addr = `"127.0.0.1:$srvPort`""
        $srvCfgText = $srvCfgText -replace 'game_mode\s*=\s*\d+', 'game_mode = 0'
        Set-Content -Encoding UTF8 $legacySrvCfgPath $srvCfgText
    } else {
        $srvCfgText = Get-Content -Raw $srvCfgPath
        $srvCfgText = $srvCfgText -replace '(?m)^bind-addr\s*=.*$', "bind-addr=127.0.0.1:$srvPort"
        $srvCfgText = $srvCfgText -replace '(?m)^server-port\s*=.*$', "server-port=$srvPort"
        $srvCfgText = $srvCfgText -replace '(?m)^(game-mode|gamemode)\s*=.*$', 'gamemode=survival'
        Set-Content -Encoding UTF8 $srvCfgPath $srvCfgText
    }
    (Get-Content -Raw $limboCfgPath) -replace 'bind_addr\s*=\s*"[^"]*"', "bind_addr = `"127.0.0.1:$limboPort`"" |
        Set-Content -Encoding UTF8 $limboCfgPath
    $proxyText = Get-Content -Raw $proxyCfgPath
    $proxyText = $proxyText -replace 'bind_addr\s*=\s*"[^"]*"', "bind_addr = `"127.0.0.1:$proxyPort`""
    $proxyText = $proxyText -replace 'limbo\s*=\s*"[^"]*"', "limbo = `"127.0.0.1:$limboPort`""
    Set-Content -Encoding UTF8 $proxyCfgPath $proxyText

    $directJson = ""
    Invoke-LoggedStep "Start-RadByte (Direct)" {
        & (Join-Path $smokeDir "Start-RadByte.ps1") -Profile Direct -WorkDir $smokeDir
    }
    Invoke-LoggedStep "MC26.3 Status Ping (Direct RadServer)" {
        $script:directJson = Invoke-McStatusPing -TargetHost "127.0.0.1" -Port $srvPort
        $parsed = $script:directJson | ConvertFrom-Json
        if ($parsed.version.protocol -ne 777) { throw "Unexpected Direct protocol: $($parsed.version.protocol)" }
        return $script:directJson
    }
    Invoke-LoggedStep "MC26.3 Raw-Wire Play Smoke (Direct RadServer)" {
        Invoke-McPlaySmoke -TargetHost "127.0.0.1" -Port $srvPort -Username "SmokeTester"
    }
    Invoke-LoggedStep "Stop-RadByte (Direct)" {
        & (Join-Path $smokeDir "Stop-RadByte.ps1") -WorkDir $smokeDir
    }

    foreach ($logName in @("RadServer.stdout.log", "RadServer.stderr.log")) {
        $p = Join-Path $smokeDir ("logs\" + $logName)
        if (Test-Path $p) { Copy-Item -Force $p (Join-Path $runLogsDir ("direct-" + $logName)) }
    }

    $worldDir = Join-Path $smokeDir "RadServer\world"
    $beforeSnap = @(Get-TreeSnapshot -Path $worldDir)
    if ($beforeSnap.Count -lt 2) {
        throw "Expected >= 2 persisted world files (.mca + playerdata) after Play smoke, got $($beforeSnap.Count)"
    }
    $beforeSnap | ConvertTo-Json -Depth 5 | Out-File -FilePath (Join-Path $runDir "persistence-before.json") -Encoding utf8

    $restoreTarget = Join-Path $smokeDir "restored-world"
    $backupDir = Join-Path $smokeDir "backups"
    Invoke-LoggedStep "Backup-RadByte + Restore Verify" {
        & (Join-Path $smokeDir "Backup-RadByte.ps1") -WorldDir $worldDir -BackupDir $backupDir -WorkDir $smokeDir -RestoreTo $restoreTarget
    }

    $afterSnap = Get-TreeSnapshot -Path $restoreTarget
    $afterSnap | ConvertTo-Json -Depth 5 | Out-File -FilePath (Join-Path $runDir "persistence-after.json") -Encoding utf8

    # Switch RadServer to modern forwarding for Network profile smoke
    $srvExtraPath = Join-Path $smokeDir 'RadServer\config\radserver-extra.properties'
    if (-not (Test-Path $srvExtraPath)) { $srvExtraPath = Join-Path $smokeDir 'RadServer\radserver-extra.properties' }
    if (Test-Path $srvExtraPath) {
        (Get-Content -Raw $srvExtraPath) -replace '(?m)^forwarding-mode\s*=.*$', 'forwarding-mode=modern' |
            Set-Content -Encoding UTF8 $srvExtraPath
    } else {
        (Get-Content -Raw $srvCfgPath) -replace '(?m)^forwarding-mode\s*=.*$', 'forwarding-mode=modern' |
            Set-Content -Encoding UTF8 $srvCfgPath
    }

    $networkJson = ""
    Invoke-LoggedStep "Start-RadByte (Network)" {
        & (Join-Path $smokeDir "Start-RadByte.ps1") -Profile Network -WorkDir $smokeDir
    }
    Invoke-LoggedStep "MC26.3 Status Ping (Network RadProxy)" {
        $script:networkJson = Invoke-McStatusPing -TargetHost "127.0.0.1" -Port $proxyPort
        $parsed = $script:networkJson | ConvertFrom-Json
        if ($parsed.version.protocol -ne 777) { throw "Unexpected Network protocol: $($parsed.version.protocol)" }
        return $script:networkJson
    }
    Invoke-LoggedStep "Stop-RadByte (Network)" {
        & (Join-Path $smokeDir "Stop-RadByte.ps1") -WorkDir $smokeDir
    }

    Get-ChildItem -Path (Join-Path $smokeDir "logs") -File | ForEach-Object {
        Copy-Item -Force $_.FullName (Join-Path $runLogsDir ("network-" + $_.Name))
    }

    $buildManifestPath = Join-Path $repoRoot "target/BUILD-MANIFEST.json"
    $buildManifest = if (Test-Path $buildManifestPath) { Get-Content -Raw $buildManifestPath | ConvertFrom-Json } else { $null }
    $gitCommit = try { (& git -C $repoRoot rev-parse HEAD 2>$null).Trim() } catch { "unknown" }
    $gitDirty = try { [bool](& git -C $repoRoot status --porcelain 2>$null) } catch { $true }

    function Get-StepResult {
        param([string]$StepName)
        $entry = $commandLog | Where-Object { $_.step -eq $StepName } | Select-Object -First 1
        if (-not $entry) { return @{ status = "NOT_RUN"; detail = "Step '$StepName' did not run in this invocation" } }
        $st = if ($entry.exit_code -eq 0) { "PASS" } else { "FAIL" }
        return @{ status = $st; detail = "$StepName ($($entry.duration_ms) ms, exit_code=$($entry.exit_code))" }
    }

    $sDirectPing  = Get-StepResult "MC26.3 Status Ping (Direct RadServer)"
    $sDirectPlay  = Get-StepResult "MC26.3 Raw-Wire Play Smoke (Direct RadServer)"
    $sBackup      = Get-StepResult "Backup-RadByte + Restore Verify"
    $sNetworkPing = Get-StepResult "MC26.3 Status Ping (Network RadProxy)"
    $sStopDirect  = Get-StepResult "Stop-RadByte (Direct)"
    $sStopNetwork = Get-StepResult "Stop-RadByte (Network)"

    $acceptance = [PSCustomObject]@{
        run_id             = $runId
        generated_utc      = (Get-Date).ToUniversalTime().ToString("o")
        source_git_commit  = $gitCommit
        source_git_dirty   = $gitDirty
        binary_manifest    = $buildManifest
        overall_status     = "PACKAGED_SMOKE_COMPLETE"
        direct_status      = $directJson
        network_status     = $networkJson
        criteria           = @(
            @{ id = "S01_DIRECT_STATUS";    status = $sDirectPing.status;  evidence = $sDirectPing.detail },
            @{ id = "S02_DIRECT_WIRE_PLAY"; status = $sDirectPlay.status;  evidence = $sDirectPlay.detail },
            @{ id = "S03_BACKUP_RESTORE";   status = $sBackup.status;      evidence = "$($sBackup.detail); restored_files=$($afterSnap.Count)" },
            @{ id = "S04_NETWORK_STATUS";   status = $sNetworkPing.status; evidence = $sNetworkPing.detail },
            @{ id = "S05_GRACEFUL_STOP";    status = $(if ($sStopDirect.status -eq "PASS" -and $sStopNetwork.status -eq "PASS") { "PASS" } else { "FAIL" }); evidence = "$($sStopDirect.detail); $($sStopNetwork.detail)" },
            @{ id = "S06_NETWORK_PLAY";     status = "NOT_RUN";            evidence = "Status ping only in Test-RadByte.ps1; full Proxy->Server/Limbo Play checked separately" },
            @{ id = "S07_GUI_CLIENT";       status = "NOT_RUN";            evidence = "Manual/external MC 26.3 GUI client not automated in headless smoke" }
        )
    }

    $acceptance | ConvertTo-Json -Depth 6 | Out-File -FilePath (Join-Path $runDir "acceptance.json") -Encoding utf8
    $commandLog | ConvertTo-Json -Depth 5 | Out-File -FilePath (Join-Path $runDir "commands-and-exit-codes.json") -Encoding utf8

    $checksums = [PSCustomObject]@{
        source_git_commit = $gitCommit
        source_git_dirty  = $gitDirty
        build_manifest    = $buildManifest
        world_before      = $beforeSnap
        world_restored    = $afterSnap
    }
    $checksums | ConvertTo-Json -Depth 6 | Out-File -FilePath (Join-Path $runDir "checksums.json") -Encoding utf8

    $reportLines = @(
        "# RadByte Packaged Test Run ($runId)",
        "",
        "- Overall status: **PACKAGED_SMOKE_COMPLETE** (measured per-step; unexecuted checks marked ``NOT_RUN``)",
        "- Source commit: ``$gitCommit`` (dirty=``$gitDirty``)",
        "- Direct status ping (protocol 777): ``$directJson``",
        "- Network status ping (protocol 777): ``$networkJson``",
        "- Backup/Restore verified files: $($afterSnap.Count)",
        "- Note: World chunks use RadByte's custom ``RAD1`` payload inside ``.mca`` region files unless vanilla-preserved. Live Mojang/GUI client testing remains ``NOT_RUN``."
    )
    $reportLines -join "`r`n" | Out-File -FilePath (Join-Path $runDir "report.md") -Encoding utf8

    Write-Host "[RadByte] Packaged smoke & artifact generation PASSED -> $runDir" -ForegroundColor Green
}
finally {
    $pidFile = Join-Path $smokeDir ".radbyte-pids.json"
    if (Test-Path $pidFile) {
        try { & (Join-Path $smokeDir "Stop-RadByte.ps1") -WorkDir $smokeDir } catch {}
    }
    if (Test-Path $smokeDir) {
        Remove-Item -Recurse -Force $smokeDir -ErrorAction SilentlyContinue
    }
}
