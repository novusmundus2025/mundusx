$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$installDir = Join-Path $env:USERPROFILE ".opengpu\bin"
$logDir = Join-Path $env:USERPROFILE ".opengpu\logs"
$logPath = Join-Path $logDir "update-0.1.95.log"
$releaseBase = "https://github.com/mundusx/releases/releases/download/cli-windows-v0.1.95"
$assets = @(
  @{ Name = "mundusx.exe"; Download = "mundusx-x86_64-pc-windows-msvc.exe"; Hash = "bbcb41f01fe81dd26d3595488982ae24c08c3925641c1bc5d64de582f9be56a2" },
  @{ Name = "opengpu.exe"; Download = "opengpu-x86_64-pc-windows-msvc.exe"; Hash = "255460c5434ddb321e53ba6d73e552b6d1441f6789379a36dde1f3fc7c78b900" }
)

New-Item -ItemType Directory -Path $installDir -Force | Out-Null
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
Start-Transcript -Path $logPath -Force | Out-Null
$stagingDir = Join-Path ([IO.Path]::GetTempPath()) ("mundusx-update-" + [guid]::NewGuid().ToString("N"))
$backupDir = Join-Path (Split-Path $installDir -Parent) ("before-0.1.95-" + (Get-Date -Format "yyyyMMdd-HHmmss"))

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
  if ($installedVersion -ne "mundusx 0.1.95") { throw "Installed version check failed: $installedVersion" }
  $tray = Join-Path $installDir "mundusx-tray.exe"
  if (Test-Path -LiteralPath $tray) { Start-Process -FilePath $tray }
  Write-Host "MundusX 0.1.95 installed. Backup: $backupDir"
} finally {
  Remove-Item -LiteralPath $stagingDir -Recurse -Force -ErrorAction SilentlyContinue
  Stop-Transcript | Out-Null
}
