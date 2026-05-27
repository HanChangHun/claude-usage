# Build + sign a release MSI for the in-app updater.
#
# Reads signing config from app/.env (gitignored — see .env.example).
# Run from the app/ directory:  .\release.ps1
#
# After this succeeds:
#   1. Bump the version in package.json, src-tauri/Cargo.toml,
#      src-tauri/tauri.conf.json, src-tauri/Cargo.lock
#   2. Commit + tag (vX.Y.Z) + push
#   3. Re-run this script
#   4. Create a GitHub release with the MSI, .msi.sig, and a hand-written
#      latest.json (see app/installers/latest.json for the format)

$ErrorActionPreference = "Stop"

# Locate this script's directory so it works regardless of CWD.
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $scriptDir

# Load .env
$envFile = Join-Path $scriptDir ".env"
if (-not (Test-Path $envFile)) {
    Write-Error "Missing $envFile — copy .env.example to .env and fill in."
    exit 1
}
Get-Content $envFile | ForEach-Object {
    if ($_ -match '^\s*#') { return }
    if ($_ -match '^\s*$') { return }
    if ($_ -match '^\s*([^=]+?)\s*=\s*(.*)\s*$') {
        $name = $matches[1]
        $value = $matches[2]
        Set-Item -Path "Env:$name" -Value $value
    }
}

# Tauri's `build` step only reads TAURI_SIGNING_PRIVATE_KEY (string), not
# TAURI_SIGNING_PRIVATE_KEY_PATH. Translate path → content here.
if ($env:TAURI_SIGNING_PRIVATE_KEY_PATH -and -not $env:TAURI_SIGNING_PRIVATE_KEY) {
    $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $env:TAURI_SIGNING_PRIVATE_KEY_PATH -Raw)
}

Write-Output "Signing key: $($env:TAURI_SIGNING_PRIVATE_KEY_PATH)"
Write-Output "Password set: $(if ($env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { 'yes' } else { 'no (key has no password)' })"
Write-Output ""

npm run tauri build
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Copy artifacts to installers/ with the v0.3.0 naming convention
# (lowercase, no en-US suffix) so URLs in latest.json stay short.
$bundleDir = "src-tauri\target\release\bundle\msi"
$version = (Get-Content "package.json" -Raw | ConvertFrom-Json).version
$srcMsi  = Join-Path $bundleDir "Claude Usage_${version}_x64_en-US.msi"
$srcSig  = "$srcMsi.sig"
$dstMsi  = "installers\claude-usage_${version}_x64.msi"
$dstSig  = "$dstMsi.sig"

if (-not (Test-Path "installers")) { New-Item -ItemType Directory installers | Out-Null }
Copy-Item -Force $srcMsi $dstMsi
Copy-Item -Force $srcSig $dstSig

Write-Output ""
Write-Output "Artifacts ready:"
Write-Output "  $dstMsi"
Write-Output "  $dstSig"
Write-Output ""
Write-Output "Signature (paste into latest.json):"
Get-Content $dstSig
