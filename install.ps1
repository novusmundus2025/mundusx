param(
  [string]$InstallDir = "$env:USERPROFILE\.opengpu\bin",
  [string]$ReleaseBaseUrl = "https://github.com/mundusx/mundusx/releases/latest/download",
  [switch]$AllowUnsignedLocalPreview,
  [switch]$InstallCudaRuntime,
  [switch]$Help
)

$ErrorActionPreference = "Stop"

function Show-Usage {
  @"
Usage:
  powershell -ExecutionPolicy Bypass -File .\install.ps1

Options:
  -InstallDir <path>       Directory where opengpu.exe will be installed.
  -ReleaseBaseUrl <url>    Release download base URL.
  -InstallCudaRuntime      Force CUDA llama runtime installation and pinning.
  -AllowUnsignedLocalPreview
                          Dev-only: allow missing checksum or signed manifest
                          when testing a local release preview.
  -Help                    Print this help and exit.

After this bootstrapper installs the binary, run:
  opengpu install
"@ | Write-Output
}

function Find-NvidiaGpu {
  $nvidiaSmi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
  if (-not $nvidiaSmi) {
    return $null
  }

  try {
    $line = & $nvidiaSmi.Source --query-gpu=name,memory.total --format=csv,noheader,nounits 2>$null |
      Select-Object -First 1
    if (-not $line) {
      return $null
    }

    $parts = $line -split ",", 2
    $name = $parts[0].Trim()
    $vramMb = $null
    if ($parts.Count -gt 1) {
      $parsed = 0
      if ([int]::TryParse($parts[1].Trim(), [ref]$parsed)) {
        $vramMb = $parsed
      }
    }

    return [pscustomobject]@{
      Name = $name
      VramMb = $vramMb
    }
  } catch {
    return $null
  }
}

function Get-WindowsTarget {
  if (-not $IsWindows -and $PSVersionTable.PSEdition -eq "Core") {
    throw "unsupported operating system: this bootstrapper is for Windows"
  }

  $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
  switch ($arch.ToString()) {
    "X64" { return "x86_64-pc-windows-msvc" }
    default { throw "unsupported Windows architecture: $arch" }
  }
}

function Read-ChecksumHash {
  param([string]$Path)

  $content = (Get-Content -Path $Path -Raw).Trim()
  if (-not $content) {
    throw "checksum file is empty"
  }

  return ($content -split "\s+")[0].ToUpperInvariant()
}

function Read-ReleaseManifest {
  param([string]$Path)

  try {
    return Get-Content -Path $Path -Raw | ConvertFrom-Json
  } catch {
    throw "release manifest is not valid JSON"
  }
}

function Get-OpenGpuHome {
  if ($env:OPENGPU_HOME) {
    return $env:OPENGPU_HOME
  }

  return (Join-Path $env:USERPROFILE ".opengpu")
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

  if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [string]) {
    $items = @()
    foreach ($item in $Value) {
      $items += ConvertTo-Hashtable -Value $item
    }
    return $items
  }

  return $Value
}

function Find-ManifestRuntimeAsset {
  param(
    [object]$Manifest,
    [string]$Name
  )

  if (-not $Manifest -or -not $Manifest.runtime_assets) {
    return $null
  }

  foreach ($asset in @($Manifest.runtime_assets)) {
    if ($asset.name -eq $Name) {
      return $asset
    }
  }

  return $null
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

function Copy-ReleaseFile {
  param(
    [string]$Source,
    [string]$Destination
  )

  $uri = $null
  if ([System.Uri]::TryCreate($Source, [System.UriKind]::Absolute, [ref]$uri) -and $uri.IsFile) {
    Copy-Item -LiteralPath $uri.LocalPath -Destination $Destination
    return
  }

  if (Test-Path -LiteralPath $Source) {
    Copy-Item -LiteralPath $Source -Destination $Destination
    return
  }

  Invoke-WebRequest -Uri $Source -OutFile $Destination
}

if ($Help) {
  Show-Usage
  exit 0
}

$target = Get-WindowsTarget
$gpu = Find-NvidiaGpu
$profile = if ($gpu) { "windows-x86_64-cuda" } else { "windows-x86_64-generic" }
$assetName = "opengpu-$target.exe"
$cudaRuntimeRequired = [bool]($gpu -or $InstallCudaRuntime)
$cudaRuntimeAssetName = "llama-cli-$target-cuda.exe"
$releaseBase = $ReleaseBaseUrl.TrimEnd("/")
$releaseUrl = "$releaseBase/$assetName"
$checksumUrl = "$releaseUrl.sha256"
$manifestUrl = "$releaseBase/release-manifest.json"
$signatureUrl = "$releaseBase/release-manifest.json.sig"
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("opengpu-install-" + [System.Guid]::NewGuid().ToString("N"))
$tempExe = Join-Path $tempDir $assetName
$tempChecksum = Join-Path $tempDir "$assetName.sha256"
$tempManifest = Join-Path $tempDir "release-manifest.json"
$tempSignature = Join-Path $tempDir "release-manifest.json.sig"
$tempCudaRuntime = Join-Path $tempDir $cudaRuntimeAssetName
$tempCudaRuntimeChecksum = Join-Path $tempDir "$cudaRuntimeAssetName.sha256"
$finalExe = Join-Path $InstallDir "opengpu.exe"
$finalCudaRuntime = Join-Path $InstallDir "llama-cli.exe"
$manifest = $null
$trustedRuntimePath = $null
$runtimeExpected = $null

Write-Output "MundusX Windows installer"
Write-Output "  target: $target"
Write-Output "  profile: $profile"
Write-Output "  gpu: $(if ($gpu) { $gpu.Name } else { 'none detected' })"
Write-Output "  cuda vram: $(if ($gpu -and $gpu.VramMb) { "$($gpu.VramMb) MB" } else { 'none detected' })"
Write-Output "  source: $releaseBase"
Write-Output "  asset: $assetName"
Write-Output "  cuda runtime: $(if ($cudaRuntimeRequired) { $cudaRuntimeAssetName } else { 'not required' })"
Write-Output "  install: $InstallDir"
Write-Output "  verification: $(if ($AllowUnsignedLocalPreview) { 'local preview override' } else { 'strict enterprise' })"
Write-Output ""

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

try {
  Write-Output "Fetching opengpu..."
  Copy-ReleaseFile -Source $releaseUrl -Destination $tempExe

  $expected = $null
  try {
    Copy-ReleaseFile -Source $checksumUrl -Destination $tempChecksum
    Write-Output "Verifying checksum..."
    $expected = Read-ChecksumHash -Path $tempChecksum
    $actual = (Get-FileHash -Algorithm SHA256 -Path $tempExe).Hash.ToUpperInvariant()
    if ($expected -ne $actual) {
      throw "checksum mismatch for $assetName"
    }
  } catch {
    if (-not $AllowUnsignedLocalPreview) {
      throw "strict installer verification failed: checksum unavailable or invalid for $assetName ($($_.Exception.Message))"
    }
    Write-Warning "dev-only local preview override: checksum unavailable or invalid for $assetName, continuing without verification"
  }

  try {
    Copy-ReleaseFile -Source $manifestUrl -Destination $tempManifest
    Copy-ReleaseFile -Source $signatureUrl -Destination $tempSignature
    Write-Output "Checking signed release manifest..."
    $manifest = Read-ReleaseManifest -Path $tempManifest
    if ($manifest.artifact_kind -ne "release-binary") {
      throw "release manifest artifact_kind is not release-binary"
    }
    if ($manifest.binary_name -ne $assetName) {
      throw "release manifest binary_name does not match $assetName"
    }
    if (-not $manifest.checksum_sha256) {
      throw "release manifest checksum_sha256 is empty"
    }
    if ($expected -and ($manifest.checksum_sha256.ToString().ToUpperInvariant() -ne $expected)) {
      throw "release manifest checksum does not match $assetName.sha256"
    }
    if (-not (Test-Path -LiteralPath $tempSignature)) {
      throw "release manifest signature is missing"
    }
    if ((Get-Item -LiteralPath $tempSignature).Length -le 0) {
      throw "release manifest signature is empty"
    }
  } catch {
    if (-not $AllowUnsignedLocalPreview) {
      throw "strict installer verification failed: signed release manifest unavailable or invalid ($($_.Exception.Message))"
    }
    Write-Warning "dev-only local preview override: signed release manifest unavailable or invalid, continuing without signature verification"
  }
  if ($cudaRuntimeRequired) {
    $runtimeManifestAsset = Find-ManifestRuntimeAsset -Manifest $manifest -Name $cudaRuntimeAssetName
    if (-not $runtimeManifestAsset -or -not $runtimeManifestAsset.checksum_sha256) {
      throw "strict installer verification failed: CUDA runtime asset $cudaRuntimeAssetName is missing from release manifest"
    }

    Write-Output "Fetching CUDA llama runtime..."
    Copy-ReleaseFile -Source "$releaseBase/$cudaRuntimeAssetName" -Destination $tempCudaRuntime
    Copy-ReleaseFile -Source "$releaseBase/$cudaRuntimeAssetName.sha256" -Destination $tempCudaRuntimeChecksum
    Write-Output "Verifying CUDA runtime checksum..."
    $runtimeExpected = Read-ChecksumHash -Path $tempCudaRuntimeChecksum
    $runtimeManifestChecksum = $runtimeManifestAsset.checksum_sha256.ToString().ToUpperInvariant()
    if ($runtimeManifestChecksum -ne $runtimeExpected) {
      throw "release manifest checksum does not match $cudaRuntimeAssetName.sha256"
    }
    $runtimeActual = (Get-FileHash -Algorithm SHA256 -Path $tempCudaRuntime).Hash.ToUpperInvariant()
    if ($runtimeExpected -ne $runtimeActual) {
      throw "checksum mismatch for $cudaRuntimeAssetName"
    }
  }

  Move-Item -Force -Path $tempExe -Destination $finalExe

  if ($cudaRuntimeRequired) {
    Move-Item -Force -Path $tempCudaRuntime -Destination $finalCudaRuntime
    $trustedRuntimePath = Save-TrustedRuntimePath -RuntimeName "llama_cli" -RuntimePath $finalCudaRuntime -Checksum $runtimeExpected
  }
} finally {
  Remove-Item -Recurse -Force -Path $tempDir -ErrorAction SilentlyContinue
}

Write-Output ""
Write-Output "Installed opengpu to $finalExe"
if ($cudaRuntimeRequired) {
  Write-Output "Installed CUDA llama runtime to $finalCudaRuntime"
  Write-Output "Pinned trusted runtime path in $trustedRuntimePath"
}

$pathEntries = ($env:PATH -split ";") | ForEach-Object { $_.TrimEnd("\") }
$normalizedInstallDir = (Resolve-Path -Path $InstallDir).Path.TrimEnd("\")
if ($pathEntries -notcontains $normalizedInstallDir) {
  Write-Warning "$InstallDir is not currently on PATH. Add it to PATH or run $finalExe directly."
}

Write-Output "Next: opengpu install"
