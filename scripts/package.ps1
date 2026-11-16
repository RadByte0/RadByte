# Builds release binaries and stages each service into its own isolated folder under dist/
param(
    [switch]$BuildGuests,
    [string]$DataPackage = "",
    [string]$DataSourceLabel = "external-data-package (provenance not verified)"
)
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$defaultsDir = Join-Path $root "config\templates"
foreach ($service in @(@('RadServer','rad-server'), @('RadProxy','rad-proxy'), @('RadLimbo','rad-limbo'))) {
    $binary = Join-Path $root ("dist/" + $service[0] + '/' + $service[1] + '.exe')
    if (Get-Process -Name $service[1] -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $binary }) {
        throw "Stop $($service[0]) before rebuilding dist. Use dist/scripts/Stop-RadByte.ps1."
    }
}
Push-Location $root
try {
    Write-Host "[RadByte] Building workspace in release mode..." -ForegroundColor Cyan
    $targetDirectory = Join-Path $root 'target'
    cargo build --workspace --release --locked
    if ($LASTEXITCODE -ne 0) { throw "Workspace build failed: $LASTEXITCODE" }
    if ($BuildGuests) {
        foreach ($guest in @('radon-guest-example', 'radon-guest-proxy')) {
            cargo build --manifest-path (Join-Path $root "examples/$guest/Cargo.toml") --target wasm32-unknown-unknown --release
            if ($LASTEXITCODE -ne 0) { throw "Guest build failed: $guest" }
        }
    }

    # Reuse an existing network secret when adding another service later.
    # Generate one fresh local forwarding secret for newly created dist/ configs.
    $rngBytes = New-Object byte[] 16
    [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($rngBytes)
    $freshSecret = ($rngBytes | ForEach-Object { $_.ToString("x2") }) -join ""

    foreach ($relativeConfig in @(
        'dist/RadProxy/rad.properties',
        'dist/RadProxy/radproxy.toml',
        'dist/RadLimbo/rad.properties',
        'dist/RadLimbo/radlimbo.toml',
        'dist/RadServer/config/radserver-extra.properties'
    )) {
        $existingConfig = Join-Path $root $relativeConfig
        if (Test-Path -LiteralPath $existingConfig) {
            $existingText = Get-Content -Raw -LiteralPath $existingConfig
            if ($existingText -match '(?m)^forwarding[_-]secret\s*=\s*"?([^"\r\n]+)') {
                $freshSecret = $Matches[1].Trim()
                break
            }
        }
    }
    $services = @(
        @{ Name = "RadProxy";  Bin = "rad-proxy.exe";  Template = "RadProxy\rad.properties";  Config = "rad.properties" },
        @{ Name = "RadLimbo";  Bin = "rad-limbo.exe";  Template = "RadLimbo\rad.properties";  Config = "rad.properties" },
        @{ Name = "RadServer"; Bin = "rad-server.exe"; Template = "RadServer\rad.properties"; Config = "rad.properties" }
    )

    $manifestBins = @()

    foreach ($svc in $services) {
        $dest = Join-Path $root ("dist\" + $svc.Name)
        New-Item -ItemType Directory -Force -Path $dest | Out-Null

        $srcBin = Join-Path $root ("target\release\" + $svc.Bin)
        $dstBin = Join-Path $dest $svc.Bin
        $srcHash = (Get-FileHash -Algorithm SHA256 -Path $srcBin).Hash.ToLowerInvariant()
        $dstHash = if (Test-Path -LiteralPath $dstBin) {
            (Get-FileHash -Algorithm SHA256 -Path $dstBin).Hash.ToLowerInvariant()
        } else { "" }

        if ($srcHash -ne $dstHash) {
            Copy-Item -Force $srcBin $dstBin
        }

        $hash = $srcHash
        $size = (Get-Item $dstBin).Length
        $manifestBins += [PSCustomObject]@{
            service    = $svc.Name
            file       = $svc.Bin
            sha256     = $hash
            size_bytes = $size
        }

        if ($svc.Config) {
            $srcCfg = Join-Path $defaultsDir $svc.Template
            if (-not (Test-Path $srcCfg)) {
                $srcCfg = Join-Path $defaultsDir $svc.Config
            }
            $dstCfg = Join-Path $dest $svc.Config
            $legacyServerCfg = Join-Path $dest "radserver.toml"
            if ((Test-Path $srcCfg) -and -not (Test-Path $dstCfg) -and
                -not ($svc.Name -eq "RadServer" -and (Test-Path $legacyServerCfg))) {
                $cfgText = Get-Content -Raw -Encoding UTF8 $srcCfg
                $cfgText = $cfgText -replace '(?m)^forwarding-secret=.*$', ("forwarding-secret=$freshSecret")
                $cfgText = $cfgText -replace 'forwarding_secret\s*=\s*"[^"]*"', ("forwarding_secret = `"$freshSecret`"")
                [System.IO.File]::WriteAllText($dstCfg, $cfgText, [System.Text.UTF8Encoding]::new($false))
                if ($svc.Name -eq "RadServer") {
                    $srcExtra = Join-Path $defaultsDir "radserver-extra.properties"
                    $dstExtra = Join-Path $dest "config\radserver-extra.properties"
                    $legacyExtra = Join-Path $dest "radserver-extra.properties"
                    if ((Test-Path $srcExtra) -and -not (Test-Path $dstExtra) -and -not (Test-Path $legacyExtra)) {
                        New-Item -ItemType Directory -Force -Path (Split-Path $dstExtra) | Out-Null
                        $extraText = Get-Content -Raw -Encoding UTF8 $srcExtra
                        $extraText = $extraText -replace '(?m)^forwarding-secret=.*$', ("forwarding-secret=$freshSecret")
                        [System.IO.File]::WriteAllText($dstExtra, $extraText, [System.Text.UTF8Encoding]::new($false))
                    }
                }
            }
        }
        if ($svc.Name -eq "RadServer") {
            $extraPath = Join-Path $dest 'config/radserver-extra.properties'
            if (-not (Test-Path -LiteralPath $extraPath) -and -not (Test-Path -LiteralPath (Join-Path $dest 'radserver-extra.properties'))) {
                New-Item -ItemType Directory -Force -Path (Split-Path -Parent $extraPath) | Out-Null
                $extra = Get-Content -Raw -LiteralPath (Join-Path $defaultsDir 'radserver-extra.properties')
                $extra = $extra -replace '(?m)^forwarding-secret=.*$', "forwarding-secret=$freshSecret"
                [IO.File]::WriteAllText($extraPath, $extra, [Text.UTF8Encoding]::new($false))
            }
            New-Item -ItemType Directory -Force -Path (Join-Path $dest "radons"), (Join-Path $dest "radonfigs") | Out-Null
            if (-not [string]::IsNullOrWhiteSpace($DataPackage)) {
                $resolvedDataPackage = (Resolve-Path -LiteralPath $DataPackage).Path
                $genVerScript = Join-Path $PSScriptRoot "generate_version_package.ps1"
                & $genVerScript -Profile release -OutDir (Join-Path $dest "versions\26.3") -DataDir $resolvedDataPackage -SourceLabel $DataSourceLabel -LibraryPath (Join-Path $targetDirectory "release/rad_version_26_3.dll") -BackupExisting -BackupDirectory (Join-Path $root "target/version-backups")
                if ($LASTEXITCODE -ne 0) { throw "Version packaging failed: $LASTEXITCODE" }
            } else {
                Write-Warning "No -DataPackage specified: dist/RadServer/versions/26.3 was not generated. Pass -DataPackage <path> to stage a version data package."
            }
        }
        Write-Host "[RadByte] Staged $($svc.Name) -> $dest" -ForegroundColor Green
    }

    New-Item -ItemType Directory -Force -Path (Join-Path $root "dist/scripts") | Out-Null
    $templatesDir = Join-Path $root "scripts\runtime"
    if (Test-Path $templatesDir) {
        Get-ChildItem -Path $templatesDir -File | ForEach-Object {
            Copy-Item -Force $_.FullName (Join-Path $root ("dist\scripts\" + $_.Name))
        }
    }

    $rustcVer = (& rustc --version).Trim()
    $cargoVer = (& cargo --version).Trim()
    $manifest = [PSCustomObject]@{
        local_testing_only    = $true
        data_package_provided = (-not [string]::IsNullOrWhiteSpace($DataPackage))
        built_at_utc          = (Get-Date).ToUniversalTime().ToString("o")
        rustc_version = $rustcVer
        cargo_version = $cargoVer
        binaries      = $manifestBins
    }
    $manifestPath = Join-Path $root "target\BUILD-MANIFEST.json"
    $manifest | ConvertTo-Json -Depth 5 | Out-File -FilePath $manifestPath -Encoding utf8
    Write-Host "[RadByte] Wrote $manifestPath" -ForegroundColor Green
}
finally {
    Pop-Location
}
