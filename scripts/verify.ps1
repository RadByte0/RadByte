# Full workspace verification gate: formatting, clippy, unit/E2E tests, and docs
param([string]$DataPackage = "")
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo test --workspace --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo doc --workspace --no-deps
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    if (-not [string]::IsNullOrWhiteSpace($DataPackage)) {
        $resolvedDataPackage = (Resolve-Path -LiteralPath $DataPackage).Path
        cargo build -p rad-version-26-3 --locked
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

        & (Join-Path $PSScriptRoot "generate_version_package.ps1") -Profile debug -DataDir $resolvedDataPackage
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

        cargo test -p rad-server --test headless_wire_e2e -- --ignored
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

        & (Join-Path $PSScriptRoot "package.ps1") -DataPackage $resolvedDataPackage
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } else {
        & (Join-Path $PSScriptRoot "package.ps1")
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }

    Write-Host "[RadByte] All checks passed and dist/ updated!" -ForegroundColor Green
}
finally {
    Pop-Location
}
