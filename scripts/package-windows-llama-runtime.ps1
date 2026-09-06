param(
  [string]$Tag = "b9856",
  [string]$CudaVersion = "12.4",
  [string]$OutputPath = "llama-runtime-x86_64-pc-windows-msvc-cuda.zip"
)

$ErrorActionPreference = "Stop"
$repo = "ggml-org/llama.cpp"
$headers = @{ "User-Agent" = "mundusx-release-runtime-packager" }
if ($env:GITHUB_TOKEN) {
  $headers["Authorization"] = "Bearer $env:GITHUB_TOKEN"
  $headers["X-GitHub-Api-Version"] = "2022-11-28"
}

$releaseUrl = "https://api.github.com/repos/$repo/releases/tags/$Tag"
$release = Invoke-RestMethod -Uri $releaseUrl -Headers $headers
$mainAssetName = "llama-$Tag-bin-win-cuda-$CudaVersion-x64.zip"
$cudartAssetName = "cudart-llama-bin-win-cuda-$CudaVersion-x64.zip"
$mainAsset = $release.assets | Where-Object { $_.name -eq $mainAssetName } | Select-Object -First 1
$cudartAsset = $release.assets | Where-Object { $_.name -eq $cudartAssetName } | Select-Object -First 1
if (-not $mainAsset) { throw "llama.cpp $Tag is missing $mainAssetName" }
if (-not $cudartAsset) { throw "llama.cpp $Tag is missing $cudartAssetName" }

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("mundusx-llama-release-" + [Guid]::NewGuid().ToString("N"))
$expanded = Join-Path $temp "expanded"
$bundle = Join-Path $temp "bundle"
New-Item -ItemType Directory -Force -Path $expanded,$bundle | Out-Null
try {
  $mainZip = Join-Path $temp $mainAssetName
  $cudartZip = Join-Path $temp $cudartAssetName
  Invoke-WebRequest -Uri $mainAsset.browser_download_url -Headers $headers -OutFile $mainZip
  Invoke-WebRequest -Uri $cudartAsset.browser_download_url -Headers $headers -OutFile $cudartZip
  Expand-Archive -LiteralPath $mainZip -DestinationPath $expanded -Force
  Expand-Archive -LiteralPath $cudartZip -DestinationPath $expanded -Force

  $llamaCli = Get-ChildItem -Path $expanded -Recurse -Filter "llama-cli.exe" | Select-Object -First 1
  $llamaServer = Get-ChildItem -Path $expanded -Recurse -Filter "llama-server.exe" | Select-Object -First 1
  if (-not $llamaCli) { throw "$mainAssetName did not contain llama-cli.exe" }
  if (-not $llamaServer) { throw "$mainAssetName did not contain llama-server.exe" }

  Get-ChildItem -LiteralPath $llamaCli.DirectoryName -File | Copy-Item -Destination $bundle -Force
  Get-ChildItem -Path $expanded -Recurse -File -Filter "*.dll" | Copy-Item -Destination $bundle -Force
  if (-not (Test-Path -LiteralPath (Join-Path $bundle "llama-cli.exe"))) { throw "packaged runtime is missing llama-cli.exe" }
  if (-not (Test-Path -LiteralPath (Join-Path $bundle "llama-server.exe"))) { throw "packaged runtime is missing llama-server.exe" }
  if (-not (Get-ChildItem -LiteralPath $bundle -Filter "cudart64_*.dll" | Select-Object -First 1)) { throw "packaged runtime is missing the CUDA runtime DLL" }

  $resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)
  if (Test-Path -LiteralPath $resolvedOutput) { Remove-Item -Force -LiteralPath $resolvedOutput }
  $tar = Get-Command tar.exe -ErrorAction SilentlyContinue
  if (-not $tar) { throw "tar.exe is required to create the Windows runtime release archive" }
  & $tar.Source -a -c -f $resolvedOutput -C $bundle .
  if ($LASTEXITCODE -ne 0) { throw "tar.exe failed to create the runtime archive with exit code $LASTEXITCODE" }
  $checksum = (Get-FileHash -Algorithm SHA256 -LiteralPath $resolvedOutput).Hash.ToLowerInvariant()
  Set-Content -LiteralPath "$resolvedOutput.sha256" -Value "$checksum  $([System.IO.Path]::GetFileName($resolvedOutput))`n" -NoNewline -Encoding ASCII
  Write-Output "Packaged pinned llama.cpp runtime $Tag CUDA $CudaVersion"
  Write-Output "Runtime: $resolvedOutput"
  Write-Output "SHA-256: $checksum"
} finally {
  Remove-Item -Recurse -Force -LiteralPath $temp -ErrorAction SilentlyContinue
}
