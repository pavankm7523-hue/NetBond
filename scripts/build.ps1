param([switch]$PortableOnly)
$root = Split-Path -Parent $PSScriptRoot
& "$PSScriptRoot\enter-msvc.ps1" {
  Push-Location $root
  try {
    if ($PortableOnly) { npm run build; if ($LASTEXITCODE) { throw "Frontend build failed ($LASTEXITCODE)" }; cargo build --manifest-path src-tauri/Cargo.toml --release }
    else { npm run tauri:build }
    if ($LASTEXITCODE) { throw "NetBond release build failed ($LASTEXITCODE)" }
    $portable = Join-Path $root 'release\portable'
    New-Item -ItemType Directory -Force -Path $portable | Out-Null
    Copy-Item -LiteralPath (Join-Path $root 'src-tauri\target\release\netbond.exe') -Destination (Join-Path $portable 'NetBond.exe') -Force
    Write-Host "Portable build: $portable\NetBond.exe"
  } finally { Pop-Location }
}
