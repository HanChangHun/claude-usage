# Build + sign a release MSI for the in-app updater, and generate latest.json.
#
# Reads signing config from app/.env (gitignored — see .env.example).
# Run from the app/ directory:  .\release.ps1 -Notes "What changed in this release"
#
# Full release flow:
#   1. Bump the version in these FOUR files (keep them in sync):
#        package.json
#        src-tauri/tauri.conf.json
#        src-tauri/Cargo.toml
#        src-tauri/Cargo.lock   (the [[package]] name = "claude-usage-app" entry)
#      The settings-panel version label in src/index.html is filled at runtime
#      via getVersion(), so it does NOT need bumping.
#   2. Run this script:   .\release.ps1 -Notes "..."
#        -> builds + signs the MSI, copies artifacts to installers\, and writes
#           installers\latest.json (UTF-8 without BOM, signature read from .sig).
#   3. Commit + tag (vX.Y.Z) + push.
#   4. Publish the GitHub release with all three assets:
#        gh release create vX.Y.Z --title "..." --notes "..." `
#          installers\claude-usage_X.Y.Z_x64.msi `
#          installers\claude-usage_X.Y.Z_x64.msi.sig `
#          installers\latest.json
#      (The updater endpoint serves releases/latest/download/latest.json, so
#       latest.json must ride on the newest non-draft release.)

param(
    [string]$Notes = ""
)

$ErrorActionPreference = "Stop"

# Locate this script's directory so it works regardless of CWD.
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $scriptDir

$repo = "HanChangHun/claude-usage"

# ---- Load .env (expanding $HOME and %ENV% references in values) ----
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
        $value = $matches[2].Trim()
        # Expand $HOME (a PowerShell-ism, not a Windows env var) then %ENV% refs,
        # so either `$HOME\.tauri\key` or an absolute path works in .env.
        $value = $value.Replace('$HOME', $HOME)
        $value = [System.Environment]::ExpandEnvironmentVariables($value)
        Set-Item -Path "Env:$name" -Value $value
    }
}

# Tauri's `build` step only reads TAURI_SIGNING_PRIVATE_KEY (string), not
# TAURI_SIGNING_PRIVATE_KEY_PATH. Translate path → content here.
if ($env:TAURI_SIGNING_PRIVATE_KEY_PATH -and -not $env:TAURI_SIGNING_PRIVATE_KEY) {
    if (-not (Test-Path $env:TAURI_SIGNING_PRIVATE_KEY_PATH)) {
        Write-Error "Signing key not found at $($env:TAURI_SIGNING_PRIVATE_KEY_PATH)"
        exit 1
    }
    $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $env:TAURI_SIGNING_PRIVATE_KEY_PATH -Raw)
}

Write-Output "Signing key: $($env:TAURI_SIGNING_PRIVATE_KEY_PATH)"
Write-Output "Password set: $(if ($env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { 'yes' } else { 'no (key has no password)' })"
Write-Output ""

# The built MSI is named from tauri.conf.json's version, but everything below
# keys off package.json — if they disagree, we'd silently package the PREVIOUS
# release's MSI still sitting in the bundle dir. Refuse to build out of sync.
$version = (Get-Content "package.json" -Raw | ConvertFrom-Json).version
$confVersion = (Get-Content "src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json).version
$cargoVersion = (Select-String -Path "src-tauri\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if ($confVersion -ne $version -or $cargoVersion -ne $version) {
    Write-Error "Version mismatch: package.json=$version tauri.conf.json=$confVersion Cargo.toml=$cargoVersion — bump all four files (see header)."
    exit 1
}

npm run tauri build
$buildExit = $LASTEXITCODE
# Don't leave signing secrets in the calling shell's environment.
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
if ($buildExit -ne 0) { exit $buildExit }

# Copy artifacts to installers/ with the v0.3.0 naming convention
# (lowercase, no en-US suffix) so URLs in latest.json stay short.
$bundleDir = "src-tauri\target\release\bundle\msi"
$srcMsi  = Join-Path $bundleDir "Claude Usage_${version}_x64_en-US.msi"
$srcSig  = "$srcMsi.sig"
$dstMsi  = "installers\claude-usage_${version}_x64.msi"
$dstSig  = "$dstMsi.sig"

if (-not (Test-Path "installers")) { New-Item -ItemType Directory installers | Out-Null }
Copy-Item -Force $srcMsi $dstMsi
Copy-Item -Force $srcSig $dstSig

# ---- Generate latest.json (UTF-8, NO BOM) for the in-app updater ----
# Read the signature straight from the .sig file — never transcribe it by hand;
# a single wrong character silently breaks update verification.
$sig = (Get-Content $dstSig -Raw).Trim()
$pubDate = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$url = "https://github.com/$repo/releases/download/v$version/claude-usage_${version}_x64.msi"
$manifest = [ordered]@{
    version   = $version
    notes     = $Notes
    pub_date  = $pubDate
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = $sig
            url       = $url
        }
    }
}
$json = $manifest | ConvertTo-Json -Depth 6
$latestPath = Join-Path $scriptDir "installers\latest.json"
# WriteAllText with UTF8Encoding($false) => no BOM (serde_json in the updater
# rejects a leading BOM).
[System.IO.File]::WriteAllText($latestPath, $json, (New-Object System.Text.UTF8Encoding($false)))

Write-Output ""
Write-Output "Artifacts ready:"
Write-Output "  $dstMsi"
Write-Output "  $dstSig"
Write-Output "  installers\latest.json   (version $version, pub_date $pubDate)"
if (-not $Notes) {
    Write-Output ""
    Write-Output "NOTE: -Notes was empty, so latest.json 'notes' is blank."
    Write-Output "      Re-run with -Notes ""...""  (or edit installers\latest.json, keeping it UTF-8 without BOM)."
}
Write-Output ""
Write-Output "Next:  commit + tag v$version + push, then:"
Write-Output "  gh release create v$version --title ""..."" --notes ""..."" $dstMsi $dstSig installers\latest.json"
