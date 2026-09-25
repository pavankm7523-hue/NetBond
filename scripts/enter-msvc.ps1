param([Parameter(Mandatory=$true)][scriptblock]$Action)

$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Visual Studio Build Tools 2022 are required. See README.md.' }
$vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vsPath) { throw 'The Visual C++ build workload is not installed. See README.md.' }
$devCmd = Join-Path $vsPath 'Common7\Tools\VsDevCmd.bat'
$environment = cmd.exe /d /c "`"$devCmd`" -arch=x64 -host_arch=x64 >nul && set"
foreach ($line in $environment) {
  $pair = $line -split '=', 2
  if ($pair.Length -eq 2) { Set-Item -Path "Env:$($pair[0])" -Value $pair[1] }
}
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
& $Action
if ($LASTEXITCODE) { exit $LASTEXITCODE }
