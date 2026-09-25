<#
.SYNOPSIS
    Build and validate the rusty-desktop-icons wheel.

.DESCRIPTION
    1. Creates (or reuses) a `.venv` at the repo root.
    2. Installs maturin, pytest.
    3. Runs `maturin build --release` and drops the wheel into
       `target/wheels`.
    4. Installs the wheel into a throwaway `.test-venv` so we exercise
       the packaged artefact rather than an editable install.
    5. Runs the pytest suite and the smoke example against that venv.

.PARAMETER Clean
    Delete .venv and .test-venv before running so we start from scratch.

.PARAMETER SkipTests
    Only build the wheel; skip the install/test round-trip.

.PARAMETER SkipSmoke
    Skip the smoke.py example (useful in headless CI where there is no
    desktop to animate).

.EXAMPLE
    .\scripts\build-wheel.ps1
    .\scripts\build-wheel.ps1 -Clean
    .\scripts\build-wheel.ps1 -SkipTests
#>

[CmdletBinding()]
param(
    [switch] $Clean,
    [switch] $SkipTests,
    [switch] $SkipSmoke
)

Set-StrictMode -Version Latest
# NOTE: $ErrorActionPreference is deliberately not 'Stop' here — PowerShell 5.1
# treats every write to stderr by a native tool (maturin prints its banner
# there) as a terminating error, which would abort the script mid-build.
# $LASTEXITCODE is checked explicitly after each external call.

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $repoRoot
Write-Host "==> repo root: $repoRoot" -ForegroundColor Cyan

$venv = Join-Path $repoRoot '.venv'
$testVenv = Join-Path $repoRoot '.test-venv'
$wheelDir = Join-Path $repoRoot 'target\wheels'
$pyprojectDir = Join-Path $repoRoot 'crates\rdi-python'

if ($Clean) {
    foreach ($dir in @($venv, $testVenv)) {
        if (Test-Path $dir) {
            Write-Host "==> removing $dir" -ForegroundColor Yellow
            Remove-Item -Recurse -Force $dir
        }
    }
}

if (-not (Test-Path $venv)) {
    Write-Host "==> creating build venv at $venv" -ForegroundColor Cyan
    python -m venv $venv
}

$venvPy = Join-Path $venv 'Scripts\python.exe'
$venvMaturin = Join-Path $venv 'Scripts\maturin.exe'

Write-Host "==> installing/upgrading maturin + pytest in build venv" -ForegroundColor Cyan
& $venvPy -m pip install --quiet --upgrade pip 'maturin==1.15.0' pytest
if ($LASTEXITCODE -ne 0) { throw "pip install failed" }

& $venvPy (Join-Path $repoRoot 'scripts\generate-stubs.py') --check
if ($LASTEXITCODE -ne 0) { throw "generated stubs are stale" }

Write-Host "==> building release wheel" -ForegroundColor Cyan
& $venvMaturin build --release `
    --manifest-path (Join-Path $pyprojectDir 'Cargo.toml') `
    --out $wheelDir
if ($LASTEXITCODE -ne 0) { throw "maturin build failed" }

$wheels = Get-ChildItem -Path $wheelDir -Filter 'rusty_desktop_icons-*.whl' |
    Sort-Object LastWriteTime -Descending
if (-not $wheels) { throw "no wheel produced in $wheelDir" }
$latest = $wheels[0]
Write-Host "==> built $($latest.Name)" -ForegroundColor Green

if ($SkipTests) {
    Write-Host "==> skipping tests (-SkipTests)" -ForegroundColor Yellow
    return
}

if (-not (Test-Path $testVenv)) {
    Write-Host "==> creating test venv at $testVenv" -ForegroundColor Cyan
    python -m venv $testVenv
}

$testPy = Join-Path $testVenv 'Scripts\python.exe'

Write-Host "==> installing wheel into test venv" -ForegroundColor Cyan
& $testPy -m pip install --quiet --upgrade pip pytest
if ($LASTEXITCODE -ne 0) { throw "pip install in test venv failed" }
& $testPy -m pip install --force-reinstall --quiet $latest.FullName
if ($LASTEXITCODE -ne 0) { throw "wheel install failed" }

Write-Host "==> running pytest against installed wheel" -ForegroundColor Cyan
& $testPy -m pytest (Join-Path $pyprojectDir 'tests') -v
if ($LASTEXITCODE -ne 0) { throw "pytest failed" }

if (-not $SkipSmoke) {
    Write-Host "==> running smoke.py" -ForegroundColor Cyan
    & $testPy (Join-Path $pyprojectDir 'examples\smoke.py')
    if ($LASTEXITCODE -ne 0) { throw "smoke.py failed" }
} else {
    Write-Host "==> skipping smoke.py (-SkipSmoke)" -ForegroundColor Yellow
}

Write-Host "==> done: $($latest.FullName)" -ForegroundColor Green
