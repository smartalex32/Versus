[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$OutputPath)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repositoryRoot
if (-not (Test-Path 'vendor')) { throw 'The vendor directory is required to create an offline dependency inventory.' }

$metadataJson = & cargo metadata --locked --offline --format-version 1
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed with exit code $LASTEXITCODE" }
$metadata = $metadataJson | ConvertFrom-Json
$projectManifest = [IO.Path]::GetFullPath((Join-Path $repositoryRoot 'Cargo.toml'))
$dependencies = foreach ($package in $metadata.packages) {
    if ([IO.Path]::GetFullPath($package.manifest_path) -eq $projectManifest) { continue }
    [PSCustomObject]@{ Name = $package.name; Version = $package.version; License = if ([string]::IsNullOrWhiteSpace($package.license)) { 'Not declared' } else { $package.license }; Source = if ([string]::IsNullOrWhiteSpace($package.source)) { 'Path dependency' } else { $package.source } }
}
$lines = @('# Dependency and license inventory', '', 'Generated from Cargo metadata using the vendored dependency tree. Review licenses before distribution; `Not declared` requires manual follow-up.', '', '| Package | Version | License | Source |', '| --- | --- | --- | --- |')
foreach ($dependency in $dependencies | Sort-Object Name, Version) { $lines += "| $($dependency.Name) | $($dependency.Version) | $($dependency.License) | $($dependency.Source) |" }
$outputDirectory = Split-Path -Parent $OutputPath
if ($outputDirectory) { New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null }
$lines | Set-Content -LiteralPath $OutputPath -Encoding utf8
