$root = Split-Path -Parent $PSScriptRoot
& "$PSScriptRoot\enter-msvc.ps1" {
  Push-Location $root
  try {
    npm run build
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
    Push-Location (Join-Path $root 'src-tauri')
    try { cargo test } finally { Pop-Location }
  } finally { Pop-Location }
}
