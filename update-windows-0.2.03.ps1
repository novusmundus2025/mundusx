$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$installDir = Join-Path $env:USERPROFILE ".opengpu\bin"
$logDir = Join-Path $env:USERPROFILE ".opengpu\logs"
$logPath = Join-Path $logDir "update-0.2.03.log"
$releaseBase = "https://github.com/mundusx/releases/releases/download/cli-windows-v0.2.03"
$assets = @(
  @{ Name = "mundusx.exe"; Download = "mundusx-x86_64-pc-windows-msvc.exe"; Hash = "c62faa8337f153f33aff9383c2be58bbf110a1424e54dad7c83850cb4d293c9e" },
  @{ Name = "opengpu.exe"; Download = "opengpu-x86_64-pc-windows-msvc.exe"; Hash = "6d2f1f02a0a0f368f05f8a7f48fef22a27aa5640c23709da3a11c598f9601fb4" }
)

New-Item -ItemType Directory -Path $installDir -Force | Out-Null
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
Start-Transcript -Path $logPath -Force | Out-Null
$stagingDir = Join-Path ([IO.Path]::GetTempPath()) ("mundusx-update-" + [guid]::NewGuid().ToString("N"))
$backupDir = Join-Path (Split-Path $installDir -Parent) ("before-0.2.03-" + (Get-Date -Format "yyyyMMdd-HHmmss"))

try {
  New-Item -ItemType Directory -Path $stagingDir -Force | Out-Null
  New-Item -ItemType Directory -Path $backupDir -Force | Out-Null
  foreach ($asset in $assets) {
    $downloadPath = Join-Path $stagingDir $asset.Download
    Invoke-WebRequest -Uri "$releaseBase/$($asset.Download)" -OutFile $downloadPath -UseBasicParsing
    $actual = (Get-FileHash -LiteralPath $downloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $asset.Hash) { throw "Checksum verification failed for $($asset.Download)." }
  }

  Get-CimInstance Win32_Process | Where-Object {
    $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).StartsWith([IO.Path]::GetFullPath($installDir), [StringComparison]::OrdinalIgnoreCase) -and
    $_.Name -in @("mundusx.exe", "opengpu.exe", "mundusx-tray.exe")
  } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  Start-Sleep -Milliseconds 500

  foreach ($asset in $assets) {
    $target = Join-Path $installDir $asset.Name
    if (Test-Path -LiteralPath $target) { Copy-Item -LiteralPath $target -Destination (Join-Path $backupDir $asset.Name) -Force }
    Copy-Item -LiteralPath (Join-Path $stagingDir $asset.Download) -Destination $target -Force
  }
  $installedVersion = (& (Join-Path $installDir "mundusx.exe") --version 2>&1 | Out-String).Trim()
  if ($installedVersion -ne "mundusx 0.2.03") { throw "Installed version check failed: $installedVersion" }
  $tray = Join-Path $installDir "mundusx-tray.exe"
  if (Test-Path -LiteralPath $tray) { Start-Process -FilePath $tray }
  Write-Host "MundusX 0.2.03 installed. Backup: $backupDir"
} finally {
  Remove-Item -LiteralPath $stagingDir -Recurse -Force -ErrorAction SilentlyContinue
  Stop-Transcript | Out-Null
}
