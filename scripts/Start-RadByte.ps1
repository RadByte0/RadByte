param([ValidateSet('Direct', 'Network')][string]$Profile = 'Direct')
$ErrorActionPreference = 'Stop'
$launcher = Join-Path (Split-Path -Parent $PSScriptRoot) 'dist/scripts/Start-RadByte.ps1'
if (-not (Test-Path -LiteralPath $launcher)) { throw 'Run ./scripts/Build.ps1 first to prepare dist.' }
& $launcher -Profile $Profile
