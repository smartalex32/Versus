[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repositoryRoot

if (-not (Test-Path 'package-lock.json')) { throw 'package-lock.json is required.' }
& npm.cmd ci --cache npm-cache --prefer-online --no-audit --no-fund
if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }

# Windows npm skips Linux native optional packages. Cache those explicitly so the
# same source archive can install on either release platform without a registry.
$lock = Get-Content 'package-lock.json' -Raw | ConvertFrom-Json -AsHashtable
$linuxPackages = $lock.packages.GetEnumerator() | Where-Object {
    $_.Value.os -contains 'linux' -and $_.Value.cpu -contains 'x64'
}
foreach ($package in $linuxPackages) {
    & npm.cmd cache add $package.Value.resolved --cache npm-cache --prefer-online
    if ($LASTEXITCODE -ne 0) { throw "Unable to cache $($package.Key)" }
}

& npm.cmd ci --offline --cache npm-cache --no-audit --no-fund
if ($LASTEXITCODE -ne 0) { throw 'Offline npm install failed.' }
Write-Host 'Frontend dependency cache refreshed and verified offline.'
