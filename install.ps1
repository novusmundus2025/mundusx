param(
  [string]$InstallDir = "$env:USERPROFILE\.opengpu\bin",
  [string]$ReleaseBaseUrl = "https://github.com/mundusx/mundusx/releases/latest/download",
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
$releaseBase = $ReleaseBaseUrl.TrimEnd("/")
$releaseUrl = "$releaseBase/$assetName"
$checksumUrl = "$releaseUrl.sha256"
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("opengpu-install-" + [System.Guid]::NewGuid().ToString("N"))
$tempExe = Join-Path $tempDir $assetName
$tempChecksum = Join-Path $tempDir "$assetName.sha256"
$finalExe = Join-Path $InstallDir "opengpu.exe"

Write-Output "MundusX Windows installer"
Write-Output "  target: $target"
Write-Output "  profile: $profile"
Write-Output "  gpu: $(if ($gpu) { $gpu.Name } else { 'none detected' })"
Write-Output "  cuda vram: $(if ($gpu -and $gpu.VramMb) { "$($gpu.VramMb) MB" } else { 'none detected' })"
Write-Output "  source: $releaseBase"
Write-Output "  asset: $assetName"
Write-Output "  install: $InstallDir"
Write-Output ""

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

try {
  Write-Output "Fetching opengpu..."
  Copy-ReleaseFile -Source $releaseUrl -Destination $tempExe

  try {
    Copy-ReleaseFile -Source $checksumUrl -Destination $tempChecksum
    Write-Output "Verifying checksum..."
    $expected = Read-ChecksumHash -Path $tempChecksum
    $actual = (Get-FileHash -Algorithm SHA256 -Path $tempExe).Hash.ToUpperInvariant()
    if ($expected -ne $actual) {
      throw "checksum mismatch for $assetName"
    }
  } catch {
    if ($_.Exception.Message -eq "checksum mismatch for $assetName") {
      throw
    }
    Write-Warning "checksum unavailable for $assetName, continuing without verification"
  }

  Move-Item -Force -Path $tempExe -Destination $finalExe
} finally {
  Remove-Item -Recurse -Force -Path $tempDir -ErrorAction SilentlyContinue
}

Write-Output ""
Write-Output "Installed opengpu to $finalExe"

$pathEntries = ($env:PATH -split ";") | ForEach-Object { $_.TrimEnd("\") }
$normalizedInstallDir = (Resolve-Path -Path $InstallDir).Path.TrimEnd("\")
if ($pathEntries -notcontains $normalizedInstallDir) {
  Write-Warning "$InstallDir is not currently on PATH. Add it to PATH or run $finalExe directly."
}

Write-Output "Next: opengpu install"
