param(
  [string]$BinDir = "$HOME\.opengpu\bin",
  [string]$PfxPath = "$HOME\.opengpu\signing\opengpu-dev-codesign.pfx",
  [string]$PfxPasswordFile = "$HOME\.opengpu\signing\pfx-password.txt",
  [string]$TimestampUrl = "http://timestamp.digicert.com"
)

$ErrorActionPreference = "Stop"

function Show-Usage {
  @"
Usage:
  powershell -ExecutionPolicy Bypass -File .\scripts\windows-dev-codesign.ps1 [options]

Authenticode-signs the local opengpu / opengpu-node-agent binaries so they
carry a real PE signature instead of shipping unsigned. Defaults to the
local dev self-signed cert; point -PfxPath / -PfxPasswordFile at an
org-issued certificate to sign with something Windows/EDR will actually
trust beyond this machine.

Options:
  -BinDir <path>            Directory containing the .exe files to sign (default: %USERPROFILE%\.opengpu\bin)
  -PfxPath <path>           Path to the signing certificate (.pfx)
  -PfxPasswordFile <path>   File containing the .pfx password (plaintext, local-only)
  -TimestampUrl <url>       RFC3161 timestamp server; falls back to unsigned-timestamp on failure
"@
}

if (-not (Test-Path $PfxPath)) {
  throw "signing certificate not found at $PfxPath. Generate a local dev cert first, or point -PfxPath at an org-issued certificate."
}
if (-not (Test-Path $PfxPasswordFile)) {
  throw "pfx password file not found at $PfxPasswordFile"
}

$signtool = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\" -Recurse -Filter "signtool.exe" -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -match "\\x64\\" } |
  Select-Object -First 1 -ExpandProperty FullName

if (-not $signtool) {
  throw "signtool.exe not found under the Windows Kits SDK; install the Windows SDK signing tools"
}

$password = (Get-Content $PfxPasswordFile -Raw).Trim()

$targets = Get-ChildItem $BinDir -Filter "*.exe" -ErrorAction SilentlyContinue
if (-not $targets) {
  throw "no .exe files found in $BinDir"
}

foreach ($target in $targets) {
  Write-Output "signing: $($target.FullName)"
  & $signtool sign /f $PfxPath /p $password /fd sha256 /tr $TimestampUrl /td sha256 $target.FullName
  if ($LASTEXITCODE -ne 0) {
    Write-Output "timestamp server unreachable or failed, retrying without timestamp..."
    & $signtool sign /f $PfxPath /p $password /fd sha256 $target.FullName
    if ($LASTEXITCODE -ne 0) {
      throw "signing failed for $($target.FullName)"
    }
  }
  & $signtool verify /pa $target.FullName
}

Write-Output "done."
