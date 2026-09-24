[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repositoryRoot

if (-not (Test-Path 'Cargo.lock')) { throw 'Cargo.lock is required. Run cargo generate-lockfile on a connected machine before vendoring.' }

$cargoConfig = Join-Path $repositoryRoot '.cargo/config.toml'
$backupConfig = Join-Path $repositoryRoot '.cargo/config.toml.vendor-backup'
if (Test-Path $backupConfig) { throw "Refusing to replace existing backup: $backupConfig" }

New-Item -ItemType Directory -Force -Path (Split-Path -Parent $cargoConfig) | Out-Null
if (Test-Path $cargoConfig) { Move-Item -LiteralPath $cargoConfig -Destination $backupConfig }
try {
    $generatedConfig = & cargo vendor --locked vendor
    if ($LASTEXITCODE -ne 0) { throw "cargo vendor failed with exit code $LASTEXITCODE" }
    @($generatedConfig, '', '[net]', 'offline = true', '', '[target.x86_64-pc-windows-msvc]', 'rustflags = ["-C", "target-feature=+crt-static"]') | Set-Content -LiteralPath $cargoConfig -Encoding utf8
} catch {
    if (-not (Test-Path $cargoConfig) -and (Test-Path $backupConfig)) { Move-Item -LiteralPath $backupConfig -Destination $cargoConfig }
    throw
} finally {
    if (Test-Path $backupConfig) { Remove-Item -LiteralPath $backupConfig -Force }
}
Write-Host 'Vendor tree refreshed. Verify with: cargo build --release --locked --offline'
