<#
  build-exe.ps1
  Builds the Ferric application as a Windows .exe and interactively installs
  missing Windows build prerequisites if needed.

  Usage:
    .\build-exe.ps1                 # Release build (default)
    .\build-exe.ps1 -Check          # Check prerequisites (offers install if missing)
    .\build-exe.ps1 -NoInstall      # Never prompt to install missing prerequisites
    .\build-exe.ps1 -Debug          # Debug build
    .\build-exe.ps1 -Test           # Run workspace tests first
    .\build-exe.ps1 -Clean          # Clean previous build artifacts
    .\build-exe.ps1 -Reinstall      # Force reinstall node_modules (npm ci)
    .\build-exe.ps1 -Run            # Launch Ferric after a successful build
    .\build-exe.ps1 -Help           # Show this help
#>
param(
    [switch]$Check,
    [switch]$NoInstall,
    [switch]$Debug,
    [switch]$Test,
    [switch]$Clean,
    [switch]$Reinstall,
    [switch]$Run,
    [switch]$Help
)

if ($Help) {
    Get-Help $MyInvocation.MyCommand.Path
    exit 0
}

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
$ui = Join-Path $root "ui"

# Ensure TLS 1.2 for downloads on Windows PowerShell 5.1
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch { }

$mode = if ($Debug) { "debug" } else { "release" }
$cargoArgs = @("build", "-p", "ferric-app")
if (-not $Debug) {
    $cargoArgs += "--release"
}

$isArm64 = ($env:PROCESSOR_ARCHITECTURE -eq "ARM64")
$script:NeedsReboot = $false

function Update-SessionPath {
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath    = [Environment]::GetEnvironmentVariable("Path", "User")
    $parts = @($machinePath, $userPath) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    if ($parts.Count -gt 0) {
        $env:Path = ($parts -join ";")
    }

    $extraPaths = @(
        (Join-Path $env:USERPROFILE ".cargo\bin"),
        (Join-Path $env:ProgramFiles "nodejs")
    )
    if (${env:ProgramFiles(x86)}) {
        $extraPaths += (Join-Path ${env:ProgramFiles(x86)} "nodejs")
    }
    foreach ($p in $extraPaths) {
        if ($p -and (Test-Path $p) -and ($env:Path -notlike "*$p*")) {
            $env:Path = "$p;$env:Path"
        }
    }
}

function Invoke-DownloadFile([string]$Url, [string]$OutFile) {
    Write-Host "    Downloading: $Url" -ForegroundColor DarkGray
    $prevProgress = $ProgressPreference
    $ProgressPreference = "SilentlyContinue"
    try {
        Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing
    }
    finally {
        $ProgressPreference = $prevProgress
    }
}

function Invoke-WingetInstall([string]$PackageId, [string[]]$ExtraArgs = @()) {
    $winget = Get-Command winget -ErrorAction SilentlyContinue
    if (-not $winget) {
        return $false
    }

    Write-Host "    Installing via winget ($PackageId)..." -ForegroundColor Cyan
    $wArgs = @(
        "install",
        "--id", $PackageId,
        "-e",
        "--accept-package-agreements",
        "--accept-source-agreements"
    ) + $ExtraArgs

    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & winget @wArgs
        $code = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $prev
    }

    # 0 = OK, -1978335189 (0x8A15002B) = already installed / no applicable update, 3010 = reboot required
    if ($code -eq 3010) {
        $script:NeedsReboot = $true
        return $true
    }
    if ($code -eq 0 -or $code -eq -1978335189) {
        return $true
    }

    Write-Host "    winget reported exit code $code - switching to a direct download..." -ForegroundColor Yellow
    return $false
}

function Install-NodeJs([string]$TempDir) {
    if (Invoke-WingetInstall -PackageId "OpenJS.NodeJS.LTS") {
        Update-SessionPath
        if ((Get-Command node -ErrorAction SilentlyContinue) -and (Get-Command npm -ErrorAction SilentlyContinue)) {
            return
        }
    }

    Write-Host "    Looking up the current Node.js LTS version..." -ForegroundColor Cyan
    $prevProgress = $ProgressPreference
    $ProgressPreference = "SilentlyContinue"
    try {
        $releases = Invoke-RestMethod -Uri "https://nodejs.org/dist/index.json" -UseBasicParsing
    }
    finally {
        $ProgressPreference = $prevProgress
    }
    $lts = $releases | Where-Object { $_.lts -and $_.lts -ne $false } | Select-Object -First 1
    if (-not $lts) {
        throw "Could not determine the Node.js LTS version from nodejs.org."
    }

    $arch = if ($isArm64) { "arm64" } else { "x64" }
    $ver  = $lts.version
    $msi  = Join-Path $TempDir "node-$ver-$arch.msi"
    Invoke-DownloadFile -Url "https://nodejs.org/dist/$ver/node-$ver-$arch.msi" -OutFile $msi

    Write-Host "    Starting the Node.js installer ($ver)..." -ForegroundColor Cyan
    $proc = Start-Process -FilePath "msiexec.exe" -ArgumentList @("/i", "`"$msi`"", "/qb") -Wait -PassThru
    if ($proc.ExitCode -eq 3010) {
        $script:NeedsReboot = $true
    } elseif ($proc.ExitCode -ne 0) {
        throw "Node.js installation failed (exit code $($proc.ExitCode))."
    }
    Update-SessionPath
}

function Install-Rust([string]$TempDir) {
    $ok = Invoke-WingetInstall -PackageId "Rustlang.Rustup"
    Update-SessionPath
    if ($ok -and (Get-Command cargo -ErrorAction SilentlyContinue) -and (Get-Command rustc -ErrorAction SilentlyContinue)) {
        return
    }

    $url = if ($isArm64) { "https://win.rustup.rs/aarch64" } else { "https://win.rustup.rs/x86_64" }
    $exe = Join-Path $TempDir "rustup-init.exe"
    Invoke-DownloadFile -Url $url -OutFile $exe

    Write-Host "    Starting rustup-init (interactive - press Enter for the defaults)..." -ForegroundColor Cyan
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $exe
        $code = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $prev
    }
    if ($code -ne 0) {
        throw "Rust installation failed (exit code $code)."
    }
    Update-SessionPath
}

function Install-MsvcBuildTools([string]$TempDir) {
    $workloadArgs = "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
    if (Invoke-WingetInstall -PackageId "Microsoft.VisualStudio.2022.BuildTools" -ExtraArgs @("--override", $workloadArgs)) {
        Update-SessionPath
        return
    }

    $exe = Join-Path $TempDir "vs_BuildTools.exe"
    Invoke-DownloadFile -Url "https://aka.ms/vs/17/release/vs_BuildTools.exe" -OutFile $exe

    Write-Host "    Starting the Visual Studio Build Tools installer (C++ workload preselected)..." -ForegroundColor Cyan
    Write-Host "    Note: the download is several GB and can take a few minutes." -ForegroundColor DarkGray
    $proc = Start-Process -FilePath $exe -ArgumentList @(
        "--passive", "--wait",
        "--add", "Microsoft.VisualStudio.Workload.VCTools",
        "--includeRecommended"
    ) -Wait -PassThru
    if ($proc.ExitCode -eq 3010) {
        $script:NeedsReboot = $true
    } elseif ($proc.ExitCode -ne 0) {
        throw "MSVC Build Tools installation failed (exit code $($proc.ExitCode))."
    }
    Update-SessionPath
}

function Install-WebView2([string]$TempDir) {
    if (Invoke-WingetInstall -PackageId "Microsoft.EdgeWebView2Runtime") {
        return
    }

    $exe = Join-Path $TempDir "MicrosoftEdgeWebview2Setup.exe"
    Invoke-DownloadFile -Url "https://go.microsoft.com/fwlink/p/?LinkId=2124703" -OutFile $exe

    Write-Host "    Starting the Microsoft Edge WebView2 Runtime installer..." -ForegroundColor Cyan
    $proc = Start-Process -FilePath $exe -Wait -PassThru
    if ($proc.ExitCode -eq 3010) {
        $script:NeedsReboot = $true
    } elseif ($proc.ExitCode -ne 0) {
        throw "WebView2 installation failed (exit code $($proc.ExitCode))."
    }
}

function Get-PrerequisiteStatus {
    Update-SessionPath
    $missing = [System.Collections.Generic.List[hashtable]]::new()
    $warnings = [System.Collections.Generic.List[string]]::new()
    $needsMsvcToolchain = $false

    # 1. Node.js & npm
    $nodeCmd = Get-Command node -ErrorAction SilentlyContinue
    $npmCmd  = Get-Command npm -ErrorAction SilentlyContinue
    if (-not $nodeCmd -or -not $npmCmd) {
        Write-Host "  [MISSING] Node.js / npm" -ForegroundColor Red
        $missing.Add(@{
            Id   = "node"
            Name = "Node.js LTS & npm"
            Hint = "winget install OpenJS.NodeJS.LTS | https://nodejs.org"
        })
    } else {
        $nodeVer = & node --version
        $npmVer  = & npm --version
        Write-Host "  [OK]      Node.js:  $nodeVer / npm $npmVer ($($nodeCmd.Source))" -ForegroundColor Green
    }

    # 2. Rust & Cargo
    $cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
    $rustcCmd = Get-Command rustc -ErrorAction SilentlyContinue
    if (-not $cargoCmd -or -not $rustcCmd) {
        Write-Host "  [MISSING] Rust / Cargo" -ForegroundColor Red
        $missing.Add(@{
            Id   = "rust"
            Name = "Rust & Cargo (rustup)"
            Hint = "winget install Rustlang.Rustup | https://rustup.rs"
        })
    } else {
        $cargoVer = & cargo --version
        Write-Host "  [OK]      Cargo:    $cargoVer" -ForegroundColor Green
        try {
            $rustHost = (& rustc -vV | Select-String "host:").Line
            if ($rustHost -and $rustHost -notmatch "windows-msvc") {
                $needsMsvcToolchain = $true
                $warnings.Add("Rust is not using the MSVC toolchain at the moment ($rustHost).")
            }
        } catch { }
    }

    # 3. Microsoft C++ Build Tools (MSVC)
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $hasMsvc = $false
    $vsPath = $null
    if (Test-Path $vswhere) {
        try {
            $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
            if (-not $vsPath -and $isArm64) {
                $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.ARM64 -property installationPath
            }
            if ($vsPath) { $hasMsvc = $true }
        } catch { }
    }
    if (-not $hasMsvc -and (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
        $hasMsvc = $true
        $vsPath = "cl.exe in PATH"
    }
    if ($hasMsvc) {
        Write-Host "  [OK]      MSVC C++: $vsPath" -ForegroundColor Green
    } else {
        Write-Host "  [MISSING] Microsoft C++ Build Tools (MSVC)" -ForegroundColor Red
        $missing.Add(@{
            Id   = "msvc"
            Name = "Microsoft C++ Build Tools (MSVC)"
            Hint = "winget install Microsoft.VisualStudio.2022.BuildTools | https://visualstudio.microsoft.com/visual-cpp-build-tools/"
        })
    }

    # 4. Microsoft Edge WebView2 Runtime
    $hasWv2 = (Test-Path "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application\*\msedgewebview2.exe") -or `
              (Test-Path "${env:ProgramFiles}\Microsoft\EdgeWebView\Application\*\msedgewebview2.exe") -or `
              (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\*" -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -match "^\{F3017226-" }) -or `
              (Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\*" -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -match "^\{F3017226-" }) -or `
              (Get-ItemProperty "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\*" -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -match "^\{F3017226-" })

    if ($hasWv2) {
        Write-Host "  [OK]      WebView2: installed" -ForegroundColor Green
    } else {
        Write-Host "  [MISSING] Microsoft Edge WebView2 Runtime" -ForegroundColor Red
        $missing.Add(@{
            Id   = "webview2"
            Name = "Microsoft Edge WebView2 Runtime"
            Hint = "winget install Microsoft.EdgeWebView2Runtime | https://developer.microsoft.com/microsoft-edge/webview2/"
        })
    }

    return @{
        Missing            = $missing
        Warnings           = $warnings
        NeedsMsvcToolchain = $needsMsvcToolchain
    }
}

function Install-MissingPrerequisites([System.Collections.Generic.List[hashtable]]$Missing) {
    Write-Host ""
    Write-Host "$($Missing.Count) prerequisite(s) for the Windows build are missing:" -ForegroundColor Yellow
    $idx = 1
    foreach ($item in $Missing) {
        Write-Host "  $idx) $($item.Name)" -ForegroundColor Yellow
        $idx++
    }

    Write-Host ""
    Write-Host "Download and install them now?" -ForegroundColor Cyan
    $choice = Read-Host "  [A] Install all  /  [E] Ask for each  /  [N] Cancel (default: A)"
    $choice = if ([string]::IsNullOrWhiteSpace($choice)) { "a" } else { $choice.Trim().ToLower() }

    if ($choice -ne "a" -and $choice -ne "e") {
        return $false
    }

    # Install MSVC before Rust so rustup detects MSVC automatically if both are missing
    $order = @("msvc", "node", "rust", "webview2")
    $ordered = foreach ($id in $order) {
        $Missing | Where-Object { $_.Id -eq $id }
    }

    $tempDir = Join-Path $env:TEMP ("ferric-setup-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    try {
        foreach ($item in $ordered) {
            if ($choice -eq "e") {
                $ans = Read-Host "`n$($item.Name): download and install now? [Y/n]"
                if ($ans -and $ans.Trim().ToLower() -notin @("y", "yes")) {
                    Write-Host "  Skipped: $($item.Name)" -ForegroundColor DarkYellow
                    continue
                }
            }

            Write-Host ""
            Write-Host "==> Installing $($item.Name)..." -ForegroundColor Cyan
            switch ($item.Id) {
                "node"     { Install-NodeJs -TempDir $tempDir }
                "rust"     { Install-Rust -TempDir $tempDir }
                "msvc"     { Install-MsvcBuildTools -TempDir $tempDir }
                "webview2" { Install-WebView2 -TempDir $tempDir }
            }
        }
    }
    finally {
        if (Test-Path $tempDir) {
            Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    return $true
}

function Test-WindowsPrerequisites([switch]$CheckOnly) {
    Write-Host "==> Checking Windows build prerequisites..." -ForegroundColor Cyan
    $status = Get-PrerequisiteStatus

    if ($status.Missing.Count -gt 0) {
        if (-not $NoInstall) {
            $installed = Install-MissingPrerequisites -Missing $status.Missing
            if ($installed) {
                Write-Host ""
                Write-Host "==> Checking prerequisites again..." -ForegroundColor Cyan
                $status = Get-PrerequisiteStatus
            }
        }
    }

    if ($status.Warnings.Count -gt 0) {
        Write-Host ""
        foreach ($w in $status.Warnings) {
            Write-Host "[Warning] $w" -ForegroundColor Yellow
        }
        if ($status.NeedsMsvcToolchain -and (-not $NoInstall) -and (Get-Command rustup -ErrorAction SilentlyContinue)) {
            $target = if ($isArm64) { "stable-aarch64-pc-windows-msvc" } else { "stable-x86_64-pc-windows-msvc" }
            $ans = Read-Host "Switch to the recommended MSVC toolchain ($target)? [Y/n]"
            if ([string]::IsNullOrWhiteSpace($ans) -or $ans.Trim().ToLower() -in @("y", "yes")) {
                & rustup default $target
            }
        }
    }

    if ($status.Missing.Count -gt 0) {
        Write-Host ""
        Write-Host "ERROR: These components are still missing:" -ForegroundColor Red
        $i = 1
        foreach ($m in $status.Missing) {
            Write-Host "  $i) $($m.Name)" -ForegroundColor Red
            Write-Host "     $($m.Hint)" -ForegroundColor DarkGray
            $i++
        }
        if ($script:NeedsReboot) {
            Write-Host ""
            Write-Host "Note: at least one installer requires a Windows restart. Please restart and run the script again." -ForegroundColor Yellow
        }
        throw "Prerequisites not met."
    }

    if ($script:NeedsReboot) {
        Write-Host ""
        Write-Host "[Note] An installer reported that a restart is recommended. If the build fails, please restart Windows." -ForegroundColor Yellow
    }

    Write-Host "All Windows build prerequisites are met.`n" -ForegroundColor Green

    if ($CheckOnly) {
        exit 0
    }
}

function Invoke-Step($label, [scriptblock]$action) {
    Write-Host ""
    Write-Host "==> $label" -ForegroundColor Cyan
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $action 2>&1 | ForEach-Object { Write-Host "$_" }
        $code = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $prev
    }
    if ($code -ne 0) { throw "$label failed (exit code $code)." }
}

try {
    # Check (and interactively install) system prerequisites
    Test-WindowsPrerequisites -CheckOnly:$Check

    # Check if a running instance locks the executable
    $running = Get-Process "ferric-app" -ErrorAction SilentlyContinue
    if ($running) {
        Write-Host "Stopping running ferric-app instance(s) before build..." -ForegroundColor Yellow
        $running | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 300
    }

    # Optional: Clean
    if ($Clean) {
        Invoke-Step "Cleaning previous builds" {
            $dist = Join-Path $ui "dist"
            if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
            cargo clean -p ferric-app
        }
    }

    # Optional: Run workspace tests first
    if ($Test) {
        Push-Location $root
        try {
            Invoke-Step "Running workspace tests (cargo test --workspace)" {
                cargo test --workspace
            }
        }
        finally {
            Pop-Location
        }
    }

    # UI Build
    Push-Location $ui
    try {
        if ($Reinstall -or (-not (Test-Path "node_modules"))) {
            if (Test-Path "package-lock.json") {
                Invoke-Step "npm ci (UI dependencies)" { npm ci }
            }
            else {
                Invoke-Step "npm install (UI dependencies)" { npm install }
            }
        }
        else {
            Write-Host "node_modules present, skipping install."
        }
        Invoke-Step "Building UI (npm run build)" { npm run build }
        if (-not (Test-Path "dist\index.html")) { throw "ui\dist\index.html missing after UI build." }
    }
    finally {
        Pop-Location
    }

    # Rust / Tauri Build
    Push-Location $root
    try {
        $argsStr = $cargoArgs -join ' '
        Invoke-Step "Rust build ($mode - cargo $argsStr)" {
            & cargo @cargoArgs
        }
    }
    finally {
        Pop-Location
    }

    $exe = Join-Path $root "target\$mode\ferric-app.exe"
    if (-not (Test-Path $exe)) { throw "The file $exe was not created." }
    $sizeMb = [math]::Round((Get-Item $exe).Length / 1MB, 1)
    Write-Host ""
    Write-Host "Done: $exe ($sizeMb MB)" -ForegroundColor Green

    # Optional: Launch
    if ($Run) {
        Write-Host ""
        Write-Host "Launching $exe ..." -ForegroundColor Cyan
        Start-Process $exe
    }
    exit 0
}
catch {
    Write-Host ""
    Write-Host "Build aborted: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
