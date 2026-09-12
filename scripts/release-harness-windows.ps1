<#
.SYNOPSIS
Build and optionally publish the MundusX Harness Windows release without GitHub Actions.

.DESCRIPTION
Builds the Windows x64 Harness runner and one-click setup executable locally, copies
them into dist/harness-runner/windows, and writes SHA-256 checksum files. Publishing
is opt-in and uploads the staged files directly to the public mundusx/releases
repository with the GitHub CLI.

.EXAMPLE
.\scripts\release-harness-windows.ps1

.EXAMPLE
.\scripts\release-harness-windows.ps1 -Publish

.EXAMPLE
.\scripts\release-harness-windows.ps1 -NextTagOnly
#>

[CmdletBinding()]
param(
  [switch]$Publish,
  [string]$Tag,
  [string]$Repository = "mundusx/releases",
  [string]$OutputDirectory,
  [switch]$SkipBuild,
  [switch]$ReplaceAssets,
  [switch]$Prerelease,
  [switch]$NextTagOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$target = "x86_64-pc-windows-msvc"
$runnerAsset = "mundusx-harness-runner-$target.exe"
$setupAsset = "mundusx-harness-setup-windows-x86_64.exe"

if (-not $OutputDirectory) {
  $OutputDirectory = Join-Path $repoRoot "dist\harness-runner\windows"
}
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)

function Invoke-CheckedCommand {
  param(
    [Parameter(Mandatory = $true)][string]$Command,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments
  )

  & $Command @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "$Command failed with exit code $LASTEXITCODE"
  }
}

function Write-Checksum {
  param([Parameter(Mandatory = $true)][string]$Path)

  $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
  $name = Split-Path -Leaf $Path
  Set-Content -NoNewline -Encoding ascii -LiteralPath "$Path.sha256" -Value "$hash  $name"
}

function Get-NextHarnessTag {
  param([Parameter(Mandatory = $true)][string]$ReleaseRepository)

  if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "GitHub CLI is required to determine the next release tag."
  }

  $releaseJson = & gh release list --repo $ReleaseRepository --limit 1000 --json tagName
  if ($LASTEXITCODE -ne 0) {
    throw "Could not read releases from $ReleaseRepository with the current GitHub login."
  }

  $candidates = @()
  foreach ($release in @($releaseJson | ConvertFrom-Json)) {
    if ($release.tagName -match '^harness-runner-v(?<version>\d+\.\d+\.\d+)-uat\.(?<build>\d+)$') {
      $candidates += [PSCustomObject]@{
        Version = [Version]$Matches.version
        Build = [int]$Matches.build
      }
    }
  }

  if ($candidates.Count -eq 0) {
    return "harness-runner-v0.1.0-uat.1"
  }

  $latest = $candidates |
    Sort-Object -Property @{ Expression = "Version"; Descending = $true }, @{ Expression = "Build"; Descending = $true } |
    Select-Object -First 1
  return "harness-runner-v$($latest.Version)-uat.$($latest.Build + 1)"
}

if (-not [Environment]::Is64BitOperatingSystem) {
  throw "The Windows Harness release requires a 64-bit Windows host."
}
if ($Tag -and $Tag -notmatch '^harness-runner-v\d+\.\d+\.\d+(?:[-.][0-9A-Za-z.-]+)?$') {
  throw "-Tag must use the harness-runner-v<version> format, for example harness-runner-v0.1.0-uat.5."
}

$selectedTag = $Tag
if (-not $selectedTag) {
  try {
    $selectedTag = Get-NextHarnessTag -ReleaseRepository $Repository
  } catch {
    if ($Publish -or $NextTagOnly) {
      throw
    }
    Write-Warning "The next release tag could not be determined: $($_.Exception.Message)"
  }
}
if ($selectedTag) {
  Write-Host "Selected release tag: $selectedTag" -ForegroundColor Cyan
}
if ($NextTagOnly) {
  Write-Output $selectedTag
  exit 0
}

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

if (-not $SkipBuild) {
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust/Cargo is required. Install Rust from https://rustup.rs and run this script again."
  }

  Push-Location $repoRoot
  try {
    Write-Host "Building the MundusX Harness runner..." -ForegroundColor Cyan
    Invoke-CheckedCommand cargo build --release --manifest-path agents/node/Cargo.toml --features harness-runner --bin mundusx-harness-runner --target $target

    Write-Host "Building the one-click Harness setup..." -ForegroundColor Cyan
    Invoke-CheckedCommand cargo build --release --manifest-path apps/windows-installer/Cargo.toml --bin mundusx-harness-setup --target $target
  } finally {
    Pop-Location
  }

  $runnerSource = Join-Path $repoRoot "target\$target\release\mundusx-harness-runner.exe"
  $setupSource = Join-Path $repoRoot "target\$target\release\mundusx-harness-setup.exe"
  foreach ($source in @($runnerSource, $setupSource)) {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
      throw "Expected build output was not created: $source"
    }
  }

  Copy-Item -Force -LiteralPath $runnerSource -Destination (Join-Path $OutputDirectory $runnerAsset)
  Copy-Item -Force -LiteralPath $setupSource -Destination (Join-Path $OutputDirectory $setupAsset)
}

$assetPaths = @(
  (Join-Path $OutputDirectory $runnerAsset),
  (Join-Path $OutputDirectory $setupAsset)
)
foreach ($assetPath in $assetPaths) {
  if (-not (Test-Path -LiteralPath $assetPath -PathType Leaf)) {
    throw "Missing staged asset: $assetPath. Run without -SkipBuild first."
  }
  Write-Checksum -Path $assetPath
}

$uploadPaths = @(
  $assetPaths[0],
  "$($assetPaths[0]).sha256",
  $assetPaths[1],
  "$($assetPaths[1]).sha256"
)

Write-Host "Harness release assets are ready:" -ForegroundColor Green
foreach ($path in $uploadPaths) {
  $file = Get-Item -LiteralPath $path
  Write-Host ("  {0} ({1:N0} bytes)" -f $file.FullName, $file.Length)
}

if (-not $Publish) {
  Write-Host "Build complete. Nothing was uploaded." -ForegroundColor Yellow
  Write-Host "To publish with the automatically selected next tag: .\scripts\release-harness-windows.ps1 -SkipBuild -Publish"
  exit 0
}

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
  throw "GitHub CLI is required for publishing. Install it and run 'gh auth login'."
}

Invoke-CheckedCommand gh auth status
$permission = (& gh repo view $Repository --json viewerPermission --jq '.viewerPermission').Trim()
if ($LASTEXITCODE -ne 0) {
  throw "Could not inspect $Repository with the current GitHub login."
}
if ($permission -notin @("ADMIN", "MAINTAIN", "WRITE")) {
  throw "The current GitHub login has $permission permission on $Repository; WRITE or ADMIN is required."
}

& gh release view $selectedTag --repo $Repository *> $null
$releaseExists = $LASTEXITCODE -eq 0
if ($releaseExists -and -not $ReplaceAssets) {
  throw "Release $selectedTag already exists in $Repository. Choose a new tag or pass -ReplaceAssets."
}

if ($releaseExists) {
  Write-Host "Replacing assets on existing release $selectedTag..." -ForegroundColor Cyan
  Invoke-CheckedCommand gh release upload $selectedTag @uploadPaths --repo $Repository --clobber
  if (-not $Prerelease) {
    Invoke-CheckedCommand gh release edit $selectedTag --repo $Repository --latest --prerelease=false
  }
} else {
  Write-Host "Creating public release $selectedTag..." -ForegroundColor Cyan
  $releaseArguments = @(
    "release", "create", $selectedTag
  ) + $uploadPaths + @(
    "--repo", $Repository,
    "--target", "main",
    "--title", "MundusX Harness runner $selectedTag",
    "--notes", "Windows x64 Harness runner and one-click setup, built locally from the MundusX source repository. SHA-256 checksum files are included."
  )
  if ($Prerelease) {
    $releaseArguments += "--prerelease"
  } else {
    $releaseArguments += "--latest"
  }
  Invoke-CheckedCommand gh @releaseArguments
}

$releaseUrl = (& gh release view $selectedTag --repo $Repository --json url --jq '.url').Trim()
if ($LASTEXITCODE -ne 0 -or -not $releaseUrl) {
  throw "The upload finished, but the release URL could not be read."
}
Write-Host "Published: $releaseUrl" -ForegroundColor Green
