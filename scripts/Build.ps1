# Build all services and automatically prepare dist for local testing.
param(
    [switch]$BuildGuests,
    [string]$DataPackage = "",
    [string]$DataSourceLabel = "external-data-package (provenance not verified)"
)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'package.ps1') -BuildGuests:$BuildGuests -DataPackage $DataPackage -DataSourceLabel $DataSourceLabel
