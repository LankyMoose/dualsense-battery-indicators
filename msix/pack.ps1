# DualSense Battery Indicators — MSIX / Microsoft Store
#
# Builds an unsigned .msix from a release exe (Partner Center re-signs Store uploads).
# Identity defaults are for GitHub artifacts; replace Publisher with the CN from
# Partner Center before a Store submission.
#
# Usage:
#   pwsh msix/pack.ps1 -Exe target/release/dualsense-battery-indicators.exe
#   pwsh msix/pack.ps1 -Exe ... -Publisher "CN=YOUR_PARTNER_CENTER_PUBLISHER"

param(
    [Parameter(Mandatory = $true)]
    [string] $Exe,

    [string] $OutDir = "target/msix",

    [string] $Version = "",

    [string] $Publisher = "CN=LankyMoose",

    [string] $IdentityName = "LankyMoose.DualSenseBatteryIndicators",

    [string] $RepoRoot = ""
)

$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
}

if (-not [System.IO.Path]::IsPathRooted($Exe)) {
    $Exe = Join-Path $RepoRoot $Exe
}
$Exe = [System.IO.Path]::GetFullPath($Exe)
if (-not (Test-Path $Exe)) {
    throw "exe not found: $Exe"
}

if (-not $Version) {
    $cargo = Get-Content (Join-Path $RepoRoot "Cargo.toml") -Raw
    if ($cargo -match '(?m)^version\s*=\s*"([^"]+)"') {
        $Version = $Matches[1]
    } else {
        throw "could not read version from Cargo.toml"
    }
}

$parts = $Version.Split('.')
while ($parts.Count -lt 4) { $parts += '0' }
$msixVersion = "$($parts[0]).$($parts[1]).$($parts[2]).$($parts[3])"

Write-Host "Generating Store assets..."
Push-Location $RepoRoot
try {
    cargo run --quiet --release --example gen_store_assets -- (Join-Path $PSScriptRoot "Assets")
} finally {
    Pop-Location
}

if (-not [System.IO.Path]::IsPathRooted($OutDir)) {
    $OutDir = Join-Path $RepoRoot $OutDir
}

$staging = Join-Path $OutDir "staging"
if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Path (Join-Path $staging "Assets") | Out-Null

Copy-Item $Exe (Join-Path $staging "dualsense-battery-indicators.exe")
Copy-Item (Join-Path $PSScriptRoot "Assets\*") (Join-Path $staging "Assets")

$manifest = Get-Content (Join-Path $PSScriptRoot "AppxManifest.xml.template") -Raw
$manifest = $manifest.Replace("{{IDENTITY_NAME}}", $IdentityName)
$manifest = $manifest.Replace("{{PUBLISHER}}", $Publisher)
$manifest = $manifest.Replace("{{VERSION}}", $msixVersion)
$utf8 = New-Object System.Text.UTF8Encoding $false
[System.IO.File]::WriteAllText(
    (Join-Path $staging "AppxManifest.xml"),
    $manifest,
    $utf8
)

$kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
$makeappx = Get-ChildItem -Path $kits -Recurse -Filter makeappx.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.Directory.Name -eq "x64" } |
    Sort-Object { $_.Directory.Parent.Name } -Descending |
    Select-Object -First 1

if (-not $makeappx) {
    throw "makeappx.exe not found under $kits (install the Windows 10/11 SDK)."
}

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$package = Join-Path $OutDir "dualsense-battery-indicators.msix"
& $makeappx.FullName pack /d $staging /p $package /o
if ($LASTEXITCODE -ne 0) {
    throw "makeappx failed with exit $LASTEXITCODE"
}

Write-Host "Packed $package (version $msixVersion, $IdentityName / $Publisher)"
