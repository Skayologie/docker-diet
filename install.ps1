# docker-diet Windows Installer
# Usage: irm https://jawadboulmal.com/envault/install.ps1 | iex

$ErrorActionPreference = 'Stop'

# ── Config (update REPO when you push to GitHub) ──────────────────────────────
$REPO    = "Skayologie/docker-diet"          
$BIN     = "docker-diet"
$INSTALL = "$env:LOCALAPPDATA\Programs\docker-diet"
# ─────────────────────────────────────────────────────────────────────────────

function Write-Step($m) { Write-Host "`n  >> $m" -ForegroundColor Cyan }
function Write-Ok($m)   { Write-Host "     OK  $m" -ForegroundColor Green }
function Write-Warn($m) { Write-Host "     !!  $m" -ForegroundColor Yellow }
function Write-Err($m)  { Write-Host "`n  ERR: $m`n" -ForegroundColor Red; exit 1 }

Clear-Host
Write-Host ""
Write-Host "  ╔══════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "  ║     docker-diet  —  Installer        ║" -ForegroundColor Cyan
Write-Host "  ╚══════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""

# ── 1. Detect architecture ────────────────────────────────────────────────────

Write-Step "Detecting system..."

$arch = if ([System.Environment]::Is64BitOperatingSystem) {
    if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
} else {
    Write-Err "32-bit Windows is not supported."
}

$assetName = "$BIN-windows-$arch.exe"
Write-Ok "Platform: Windows $arch"

# ── 2. Fetch latest release from GitHub ───────────────────────────────────────

Write-Step "Fetching latest release..."

$apiUrl  = "https://api.github.com/repos/$REPO/releases/latest"
$headers = @{ 'User-Agent' = 'docker-diet-installer' }

try {
    $release = Invoke-RestMethod -Uri $apiUrl -Headers $headers
} catch {
    Write-Err "Could not reach GitHub API. Check your internet connection.`n  $($_.Exception.Message)"
}

$version = $release.tag_name
$asset   = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1

if (-not $asset) {
    Write-Err "No binary found for '$assetName' in release $version.`n  Available assets: $($release.assets.name -join ', ')"
}

Write-Ok "Latest version : $version"
Write-Ok "Download target: $assetName"

# ── 3. Download ───────────────────────────────────────────────────────────────

Write-Step "Downloading $BIN $version..."

$null = New-Item -ItemType Directory -Force -Path $INSTALL
$dest = "$INSTALL\$BIN.exe"
$tmp  = "$env:TEMP\$BIN-$version.exe"

try {
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmp -UseBasicParsing
} catch {
    Write-Err "Download failed: $($_.Exception.Message)"
}

# Replace existing binary and remove the "Mark of the Web" so
# SmartScreen does not block the executable after installation.
Copy-Item -Path $tmp -Destination $dest -Force
Unblock-File -Path $dest
Remove-Item $tmp -ErrorAction SilentlyContinue

Write-Ok "Installed to: $dest"

# ── 4. Add to PATH ────────────────────────────────────────────────────────────

Write-Step "Adding to PATH..."

$userPath = [System.Environment]::GetEnvironmentVariable("PATH", "User")

if ($userPath -notlike "*$INSTALL*") {
    [System.Environment]::SetEnvironmentVariable(
        "PATH", "$INSTALL;$userPath", "User"
    )
    $env:PATH = "$INSTALL;$env:PATH"
    Write-Ok "Added $INSTALL to your user PATH."
} else {
    Write-Ok "Already in PATH."
}

# ── 5. Verify ─────────────────────────────────────────────────────────────────

Write-Step "Verifying..."

try {
    $ver = & "$dest" --version 2>&1
    Write-Ok "docker-diet $ver is ready."
} catch {
    Write-Warn "Binary installed but could not verify. Try opening a new terminal."
}

# ── Done ──────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "  ✓ Installation complete!" -ForegroundColor Green
Write-Host ""
Write-Host "  Quick start:" -ForegroundColor DarkGray
Write-Host "    docker-diet --help"
Write-Host "    docker-diet dry-run  --image nginx:latest"
Write-Host "    docker-diet analyze  --image myapp:latest"
Write-Host ""
Write-Host "  NOTE: Open a new terminal if the command is not found yet." -ForegroundColor Yellow
Write-Host ""
