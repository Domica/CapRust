# Build CapRust release binary and compile the Windows installer.
#
# Usage:
#   pwsh -File installer/build.ps1
# or from any PowerShell:
#   .\installer\build.ps1
#
# Prerequisites:
#   - Inno Setup 6 (winget install JRSoftware.InnoSetup)
#
# Output:
#   installer/dist/CapRust-<version>-win64-setup.exe

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot

Write-Host "[1/5] Reading version from workspace Cargo.toml..." -ForegroundColor Cyan
$cargoToml = Get-Content (Join-Path $repoRoot 'Cargo.toml') -Raw
if ($cargoToml -notmatch 'version\s*=\s*"([^"]+)"') {
    throw "Could not parse workspace version from Cargo.toml"
}
$version = $Matches[1]
Write-Host "  version: $version"

Push-Location $repoRoot
try {
    Write-Host "[2/5] cargo build --release -p caprust-app..." -ForegroundColor Cyan
    cargo build --release -p caprust-app
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    Write-Host "[3/5] Staging release binary..." -ForegroundColor Cyan
    $candidates = @(
        $(if ($env:CARGO_TARGET_DIR) { Join-Path $env:CARGO_TARGET_DIR 'release\caprust-app.exe' }),
        (Join-Path $repoRoot 'target\release\caprust-app.exe')
    )
    # NB: Select-Object -First 1 returns a scalar (not a 1-element
    # array), so `$exe[0]` below would slice the string's first
    # character. Assign directly.
    $exe = $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
    if (-not $exe) {
        throw "caprust-app.exe not found after cargo build (checked CARGO_TARGET_DIR and target/)"
    }
    $sizeMb = [math]::Round((Get-Item $exe).Length / 1MB, 2)
    Write-Host "  built: $exe ($sizeMb MB)"

    $staging = Join-Path $PSScriptRoot 'staging'
    New-Item -ItemType Directory -Force -Path $staging | Out-Null
    Copy-Item $exe (Join-Path $staging 'caprust-app.exe') -Force

    Write-Host "[4/5] Compiling installer with Inno Setup..." -ForegroundColor Cyan
    $isccCandidates = @(
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
        'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
        'C:\Program Files\Inno Setup 6\ISCC.exe'
    )
    $iscc = $isccCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) {
        throw "ISCC.exe not found. Install Inno Setup 6 (winget install JRSoftware.InnoSetup)."
    }

    & $iscc "/DMyAppVersion=$version" (Join-Path $PSScriptRoot 'caprust.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }

    Write-Host "[5/5] Done." -ForegroundColor Green
    Get-ChildItem (Join-Path $PSScriptRoot 'dist') -Filter '*.exe' -ErrorAction SilentlyContinue |
        ForEach-Object {
            $mb = [math]::Round($_.Length / 1MB, 2)
            Write-Host "  -> $($_.FullName) ($mb MB)"
        }
}
finally {
    Pop-Location
}
