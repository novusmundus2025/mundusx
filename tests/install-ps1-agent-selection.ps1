$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$installerPath = Join-Path $repoRoot "install.ps1"
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
  $installerPath,
  [ref]$tokens,
  [ref]$parseErrors
)
if ($parseErrors.Count -gt 0) {
  throw "install.ps1 contains PowerShell parse errors"
}

$functionAst = $ast.Find({
  param($node)
  $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
    $node.Name -eq "Select-AgentMode"
}, $true)
if (-not $functionAst) {
  throw "Select-AgentMode was not found"
}

$installerSource = Get-Content -LiteralPath $installerPath -Raw
if ($installerSource -notmatch '\[string\]\$Workspace\s*=\s*""') {
  throw "installer must accept an optional workspace"
}
if ($installerSource -notmatch 'Join-Path \$env:USERPROFILE "MundusX\\Projects"') {
  throw "installer must use the Windows home workspace default"
}
if ($installerSource -notmatch '\$AgentMode -ne "none" -and -not \$SkipChatConnect') {
  throw "Chat connection must run only when an agent is selected"
}
if ($installerSource -notmatch '\[switch\]\$SkipModelRuntime') {
  throw "installer must expose an agent-only profile without a model runtime"
}
if ($installerSource -notmatch 'no llama\.cpp server, local model, CUDA, or Vulkan runtime') {
  throw "agent-only profile must explicitly omit local inference components"
}

$selector = $functionAst.Body.GetScriptBlock()
try {
  function global:Read-Host { return "1" }
  if ((& $selector) -ne "hermes") { throw "choice 1 must select Hermes" }

  function global:Read-Host { return "2" }
  if ((& $selector) -ne "native") { throw "choice 2 must select the native agent" }

  function global:Read-Host { return "3" }
  if ((& $selector) -ne "none") { throw "choice 3 must defer agent setup" }

  function global:Read-Host { return $null }
  if ((& $selector) -ne "none") { throw "empty input must defer agent setup" }
} finally {
  Remove-Item -Path Function:\Read-Host -ErrorAction SilentlyContinue
}

Write-Output "PASS: install.ps1 offers Hermes, native, and deferred agent setup"
