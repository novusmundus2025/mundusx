param(
  [string]$Repo = "ggml-org/llama.cpp",
  [string]$Tag = "latest",
  [string]$CudaVersion,
  [switch]$SkipBuild,
  [switch]$SkipHealthCheck
)

$ErrorActionPreference = "Stop"

function Show-Usage {
  @"
Usage:
  powershell -ExecutionPolicy Bypass -File .\scripts\windows-dev-llama-runtime.ps1 [options]

Bootstraps a Windows dev machine that builds opengpu/opengpu-node-agent from
source with `cargo build` instead of running install.ps1. That build path
never fetches the CUDA llama.cpp runtime or pins it in
trusted-runtime-paths.json, so opengpu-node-agent health reports
llamaCliAvailable: no / policyAllowed: no until this script runs.

This script:
  1. builds opengpu and opengpu-node-agent in release mode (unless -SkipBuild)
  2. detects the NVIDIA driver's supported CUDA version (unless -CudaVersion is given)
  3. downloads the matching llama.cpp Windows CUDA release + cudart runtime zip
  4. extracts llama-cli.exe and its CUDA DLLs into `<OPENGPU_HOME>\runtimes\llama`
  5. pins the checksum in `<OPENGPU_HOME>\trusted-runtime-paths.json`
  6. runs `opengpu-node-agent health` to confirm the fix (unless -SkipHealthCheck)

Options:
  -Repo <owner/name>     llama.cpp repo to pull releases from (default: ggml-org/llama.cpp)
  -Tag <tag>             Release tag to install, e.g. b9856 (default: latest)
  -CudaVersion <x.y>     Force a CUDA asset version, e.g. 12.4 (default: auto-detected from nvidia-smi)
  -SkipBuild             Skip the cargo build step
  -SkipHealthCheck       Skip running opengpu-node-agent health at the end
"@ | Write-Output
}

function Get-OpenGpuHome {
  if ($env:OPENGPU_HOME) {
    return $env:OPENGPU_HOME
  }
  return (Join-Path $env:USERPROFILE ".opengpu")
}

function Get-CargoExe {
  $cargo = Get-Command cargo -ErrorAction SilentlyContinue
  if ($cargo) {
    return $cargo.Source
  }
  $fallback = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
  if (Test-Path -LiteralPath $fallback) {
    return $fallback
  }
  throw "cargo not found on PATH or at $fallback"
}

function Get-DriverCudaVersion {
  $nvidiaSmi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
  if (-not $nvidiaSmi) {
    return $null
  }
  $output = & $nvidiaSmi.Source 2>$null
  $match = $output | Select-String -Pattern "CUDA Version:\s*([0-9]+\.[0-9]+)"
  if ($match) {
    return $match.Matches[0].Groups[1].Value
  }
  return $null
}

function Select-CudaAssetVersion {
  param([string]$DriverCudaVersion)

  if ($CudaVersion) {
    return $CudaVersion
  }
  if (-not $DriverCudaVersion) {
    Write-Warning "could not detect NVIDIA driver CUDA version; defaulting to 12.4. Pass -CudaVersion to override."
    return "12.4"
  }

  $parsed = [double]$DriverCudaVersion
  if ($parsed -ge 13.0) {
    return "13.3"
  }
  return "12.4"
}

function ConvertTo-Hashtable {
  param([object]$Value)

  if ($null -eq $Value) {
    return $null
  }
  if ($Value -is [System.Collections.IDictionary]) {
    $table = [ordered]@{}
    foreach ($key in $Value.Keys) {
      $table[$key] = ConvertTo-Hashtable -Value $Value[$key]
    }
    return $table
  }
  if ($Value -is [System.Management.Automation.PSCustomObject]) {
    $table = [ordered]@{}
    foreach ($property in $Value.PSObject.Properties) {
      $table[$property.Name] = ConvertTo-Hashtable -Value $property.Value
    }
    return $table
  }
  return $Value
}

function Save-TrustedRuntimePath {
  param(
    [string]$RuntimeName,
    [string]$RuntimePath,
    [string]$Checksum
  )

  $opengpuHome = Get-OpenGpuHome
  New-Item -ItemType Directory -Force -Path $opengpuHome | Out-Null
  $trustedPath = Join-Path $opengpuHome "trusted-runtime-paths.json"
  $trusted = [ordered]@{}

  if (Test-Path -LiteralPath $trustedPath) {
    $existing = Get-Content -Path $trustedPath -Raw | ConvertFrom-Json
    $trusted = ConvertTo-Hashtable -Value $existing
  }

  $trusted[$RuntimeName] = [ordered]@{
    path = [System.IO.Path]::GetFullPath($RuntimePath)
    sha256 = $Checksum.ToLowerInvariant()
  }

  $trusted | ConvertTo-Json -Depth 8 | Set-Content -Path $trustedPath -Encoding ASCII
  return $trustedPath
}

if ($args -contains "-Help" -or $args -contains "--help" -or $args -contains "-h") {
  Show-Usage
  exit 0
}

if (-not $SkipBuild) {
  $cargo = Get-CargoExe
  Write-Output "Building opengpu and opengpu-node-agent (release)..."
  & $cargo build -p opengpu -p opengpu-node-agent --release
  if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed with exit code $LASTEXITCODE"
  }
}

$driverCudaVersion = Get-DriverCudaVersion
$cudaAssetVersion = Select-CudaAssetVersion -DriverCudaVersion $driverCudaVersion

Write-Output "Repo: $Repo"
Write-Output "Tag: $Tag"
Write-Output "Detected driver CUDA version: $(if ($driverCudaVersion) { $driverCudaVersion } else { 'unknown' })"
Write-Output "Using llama.cpp CUDA asset version: $cudaAssetVersion"

$releaseApiUrl = if ($Tag -eq "latest") {
  "https://api.github.com/repos/$Repo/releases/latest"
} else {
  "https://api.github.com/repos/$Repo/releases/tags/$Tag"
}

Write-Output "Looking up release metadata: $releaseApiUrl"
$release = Invoke-RestMethod -Uri $releaseApiUrl -Headers @{ "User-Agent" = "mundusx-dev-script" }
$resolvedTag = $release.tag_name

$mainAssetName = "llama-$resolvedTag-bin-win-cuda-$cudaAssetVersion-x64.zip"
$cudartAssetName = "cudart-llama-bin-win-cuda-$cudaAssetVersion-x64.zip"

$mainAsset = $release.assets | Where-Object { $_.name -eq $mainAssetName } | Select-Object -First 1
$cudartAsset = $release.assets | Where-Object { $_.name -eq $cudartAssetName } | Select-Object -First 1

if (-not $mainAsset) {
  throw "release $resolvedTag does not have an asset named $mainAssetName; pass -CudaVersion to pick a different build"
}
if (-not $cudartAsset) {
  throw "release $resolvedTag does not have an asset named $cudartAssetName; pass -CudaVersion to pick a different build"
}

$runtimeDir = Join-Path (Get-OpenGpuHome) "runtimes\llama"
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("llama-cuda-runtime-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

try {
  $mainZip = Join-Path $tempDir $mainAssetName
  $cudartZip = Join-Path $tempDir $cudartAssetName

  Write-Output "Downloading $mainAssetName..."
  Invoke-WebRequest -Uri $mainAsset.browser_download_url -OutFile $mainZip
  Write-Output "Downloading $cudartAssetName..."
  Invoke-WebRequest -Uri $cudartAsset.browser_download_url -OutFile $cudartZip

  if (Test-Path -LiteralPath $runtimeDir) {
    Remove-Item -Recurse -Force -LiteralPath $runtimeDir
  }
  New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null

  Write-Output "Extracting runtime into $runtimeDir..."
  Expand-Archive -Path $mainZip -DestinationPath $runtimeDir -Force
  Expand-Archive -Path $cudartZip -DestinationPath $runtimeDir -Force
} finally {
  Remove-Item -Recurse -Force -Path $tempDir -ErrorAction SilentlyContinue
}

$llamaCliPath = Join-Path $runtimeDir "llama-cli.exe"
if (-not (Test-Path -LiteralPath $llamaCliPath)) {
  $found = Get-ChildItem -Path $runtimeDir -Recurse -Filter "llama-cli.exe" | Select-Object -First 1
  if (-not $found) {
    throw "downloaded runtime bundle did not contain llama-cli.exe"
  }
  $llamaCliPath = $found.FullName
}

$checksum = (Get-FileHash -Algorithm SHA256 -Path $llamaCliPath).Hash.ToLowerInvariant()
$trustedPath = Save-TrustedRuntimePath -RuntimeName "llama_cli" -RuntimePath $llamaCliPath -Checksum $checksum

$llamaServerPath = Join-Path $runtimeDir "llama-server.exe"
if (-not (Test-Path -LiteralPath $llamaServerPath)) {
  $foundServer = Get-ChildItem -Path $runtimeDir -Recurse -Filter "llama-server.exe" | Select-Object -First 1
  if ($foundServer) {
    $llamaServerPath = $foundServer.FullName
  }
}
if (Test-Path -LiteralPath $llamaServerPath) {
  $serverChecksum = (Get-FileHash -Algorithm SHA256 -Path $llamaServerPath).Hash.ToLowerInvariant()
  $trustedPath = Save-TrustedRuntimePath -RuntimeName "llama_server" -RuntimePath $llamaServerPath -Checksum $serverChecksum
}

Write-Output ""
Write-Output "Installed llama-cli.exe to $llamaCliPath"
Write-Output "Checksum: $checksum"
if (Test-Path -LiteralPath $llamaServerPath) {
  Write-Output "Installed llama-server.exe to $llamaServerPath"
  Write-Output "Server checksum: $serverChecksum"
}
Write-Output "Pinned trusted runtime path in $trustedPath"

if (-not $SkipHealthCheck) {
  $repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
  $agentExe = Join-Path $repoRoot "target\release\opengpu-node-agent.exe"
  if (-not (Test-Path -LiteralPath $agentExe)) {
    $agentExe = (Get-Command opengpu-node-agent -ErrorAction SilentlyContinue).Source
  }
  if ($agentExe) {
    Write-Output ""
    Write-Output "Running $agentExe health ..."
    & $agentExe health
  } else {
    Write-Warning "could not find opengpu-node-agent.exe to run a health check; build it first or pass -SkipHealthCheck"
  }
}
