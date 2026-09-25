$root = Split-Path -Parent $PSScriptRoot
& "$PSScriptRoot\enter-msvc.ps1" {
  Push-Location $root
  try { npm run tauri:dev } finally { Pop-Location }
}
