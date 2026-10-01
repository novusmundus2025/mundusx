$ErrorActionPreference = "Stop"
$path = Join-Path (Split-Path -Parent $PSScriptRoot) "install.ps1"
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
$setup = $ast.Find({ param($node)
  $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq "Start-ContributorSetup"
}, $true)
Invoke-Expression $setup.Extent.Text
function Start-Process { param($FilePath, $ArgumentList) $script:captured = $ArgumentList }
$Connection = "direct"
$ClusterUrl = "http://127.0.0.1:1234/v1"
$ClusterModel = "model's name"
Start-ContributorSetup -CliPath $path
$decoded = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($script:captured[-1]))
if (-not $decoded.Contains("--connection 'direct' --cluster-url 'http://127.0.0.1:1234/v1' --cluster-model 'model''s name'")) {
  throw "Connection options were not quoted and forwarded correctly"
}
[void][System.Management.Automation.Language.Parser]::ParseInput($decoded, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
Write-Output "PASS: Windows setup forwards quoted endpoint and model options"
