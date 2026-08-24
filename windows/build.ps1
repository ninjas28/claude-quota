<#
.SYNOPSIS
    Builds claude-quota.exe. Installing is install.ps1's job; this only ever builds.

.DESCRIPTION
    The macOS counterpart is menubar/build.sh, which assembles an .app bundle.
    There is no bundle to assemble here -- the result is one self-contained exe.

    One wrinkle worth knowing about: a Rust target directory is tens of
    thousands of files and a couple of gigabytes. If the repository lives inside
    OneDrive (or Dropbox, or Google Drive), that whole tree gets queued for
    upload on every build, which is slow, fills the drive, and produces sync
    conflicts on files that change every compile. When that is the case this
    script builds to LOCALAPPDATA instead and says so. Override with -TargetDir,
    or set CARGO_TARGET_DIR yourself.

.PARAMETER Dev
    Build the debug profile instead of release. Faster to compile, much larger.
    Named -Dev rather than -Debug because -Debug is a PowerShell common parameter
    and redeclaring it is an error.

.PARAMETER PathOnly
    Resolve and print where the binary would be, without building. Lets
    install.ps1 -SkipBuild find the last build without having to duplicate the
    target-directory rules above.

.PARAMETER TargetDir
    Where to put build artifacts. Defaults to CARGO_TARGET_DIR, then to the
    out-of-sync-folder location described above, then to ./target.
#>
[CmdletBinding()]
param(
    [switch] $Dev,
    [switch] $PathOnly,
    [string] $TargetDir
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "cargo not found. Install Rust from https://rustup.rs and re-run."
}

function Resolve-TargetDir {
    if ($TargetDir) { return $TargetDir }
    if ($env:CARGO_TARGET_DIR) { return $env:CARGO_TARGET_DIR }

    $synced = @('OneDrive', 'Dropbox', 'Google Drive', 'iCloudDrive')
    foreach ($name in $synced) {
        if ($PSScriptRoot -like "*$name*") {
            $fallback = Join-Path $env:LOCALAPPDATA 'claude-quota-build'
            if (-not $PathOnly) {
                Write-Host "==> Repository is inside $name; building to $fallback instead" -ForegroundColor Yellow
                Write-Host "    (a Rust target directory is ~2 GB of constantly-changing files)"
            }
            return $fallback
        }
    }
    return (Join-Path $PSScriptRoot 'target')
}

$resolved = Resolve-TargetDir
$env:CARGO_TARGET_DIR = $resolved
$profileName = if ($Dev) { 'debug' } else { 'release' }

$exe = Join-Path $resolved "$profileName\claude-quota.exe"

if ($PathOnly) {
    return $exe
}

Write-Host "==> Building ($profileName)"
if ($Dev) { cargo build } else { cargo build --release }
if ($LASTEXITCODE -ne 0) { Write-Error "cargo build failed" }
if (-not (Test-Path $exe)) { Write-Error "build succeeded but no binary at $exe" }

Write-Host "==> Built: $exe" -ForegroundColor Green
$exe
