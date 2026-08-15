param(
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$repository = 'sunaemon/keymap-overlay'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runValue = 'KeymapOverlay'

function Main {
    param([switch]$Remove)

    Initialize-Paths
    if ($Remove) {
        Uninstall-Release
        return
    }

    Install-Release
}

function Install-Release {
    Assert-SupportedPlatform
    Assert-LayerAssets

    $temporaryDirectory = Join-Path $env:TEMP "keymap-overlay-$([guid]::NewGuid())"
    New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null

    try {
        $release = Stage-Release -TemporaryDirectory $temporaryDirectory
        $backupDirectory = Join-Path $temporaryDirectory 'backup'
        Backup-Installation -BackupDirectory $backupDirectory
        Stop-Overlay

        try {
            Install-StagedFiles -TemporaryDirectory $temporaryDirectory
            Install-Autostart
        }
        catch {
            $installationError = $_.Exception.Message
            $rollbackErrors = @()

            try { Stop-Overlay } catch { $rollbackErrors += "stopping the overlay: $($_.Exception.Message)" }
            try { Restore-Installation -BackupDirectory $backupDirectory } catch { $rollbackErrors += "restoring files: $($_.Exception.Message)" }
            try { Restart-PreviousInstallation } catch { $rollbackErrors += "restarting the previous installation: $($_.Exception.Message)" }

            if ($rollbackErrors.Count -gt 0) {
                throw "Installation failed: $installationError. Rollback also failed while $($rollbackErrors -join '; ')."
            }
            throw "Installation failed and the previous installation was restored: $installationError"
        }

        Write-InstalledFiles -ReleaseTag $release
    }
    finally {
        Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Uninstall-Release {
    Stop-Overlay
    Remove-ItemProperty -Path $runKey -Name $runValue -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $binaryPath, $licensePath, $thirdPartyLicensesPath, $installerPath -Force -ErrorAction SilentlyContinue

    Write-Output 'Removed:'
    Write-Output "  binary: $binaryPath"
    Write-Output "  licenses: $licensePath, $thirdPartyLicensesPath"
    Write-Output "  installer: $installerPath"
    Write-Output "  autostart: $runKey\$runValue"
    Write-Output "Kept layer models: $assetDirectory"
    Write-Output "Kept logs: $logDirectory"
}

function Initialize-Paths {
    $script:assetDirectory = Join-Path $env:USERPROFILE '.config\keymap-overlay'
    $script:binaryPath = Join-Path $assetDirectory 'keymap-overlay.exe'
    $script:licensePath = Join-Path $assetDirectory 'LICENSE'
    $script:thirdPartyLicensesPath = Join-Path $assetDirectory 'THIRD-PARTY-LICENSES.html'
    $script:installerPath = Join-Path $assetDirectory 'install.ps1'
    $script:logDirectory = Join-Path $env:USERPROFILE '.local\var\log\keymap-overlay'
}

function Assert-SupportedPlatform {
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        throw "No release binary is available for Windows $env:PROCESSOR_ARCHITECTURE."
    }
}

function Assert-LayerAssets {
    if (-not (Get-ChildItem -LiteralPath $assetDirectory -Filter '*_L*.json' -File -ErrorAction SilentlyContinue)) {
        throw "No layer JSON models found in $assetDirectory. Generate assets from a source checkout before installing the binary."
    }
}

function Stage-Release {
    param([string]$TemporaryDirectory)

    $releaseMetadata = Invoke-RestMethod -Uri "https://api.github.com/repos/$repository/releases/latest"
    $releaseTag = $releaseMetadata.tag_name
    if ($releaseTag -notmatch '^v\d+\.\d+\.\d+$') {
        throw "Latest release returned invalid tag '$releaseTag'."
    }

    $assetName = 'keymap-overlay-windows-x86_64.zip'
    $archivePath = Join-Path $TemporaryDirectory $assetName
    $checksumsPath = Join-Path $TemporaryDirectory 'SHA256SUMS'
    $stagedInstallerPath = Join-Path $TemporaryDirectory 'release-install.ps1'
    $releaseUrl = "https://github.com/$repository/releases/download/$releaseTag"
    Invoke-WebRequest -Uri "$releaseUrl/$assetName" -OutFile $archivePath
    Invoke-WebRequest -Uri "$releaseUrl/SHA256SUMS" -OutFile $checksumsPath
    Invoke-WebRequest -Uri "$releaseUrl/install.ps1" -OutFile $stagedInstallerPath
    Confirm-Checksum -ArchivePath $archivePath -ChecksumsPath $checksumsPath -AssetName $assetName
    Confirm-Checksum -ArchivePath $stagedInstallerPath -ChecksumsPath $checksumsPath -AssetName 'install.ps1'
    Confirm-AttestationsIfAvailable -Paths @($archivePath, $checksumsPath, $stagedInstallerPath)
    Expand-Archive -LiteralPath $archivePath -DestinationPath $TemporaryDirectory

    foreach ($name in @('keymap-overlay.exe', 'LICENSE', 'THIRD-PARTY-LICENSES.html')) {
        if (-not (Test-Path -LiteralPath (Join-Path $TemporaryDirectory $name) -PathType Leaf)) {
            throw "$assetName does not contain $name."
        }
    }

    return $releaseTag
}

function Confirm-Checksum {
    param(
        [string]$ArchivePath,
        [string]$ChecksumsPath,
        [string]$AssetName
    )

    $escapedName = [regex]::Escape($AssetName)
    $line = Get-Content -LiteralPath $ChecksumsPath | Where-Object { $_ -match "^[0-9a-fA-F]{64}\s+\*?$escapedName$" } | Select-Object -First 1
    if (-not $line) {
        throw "SHA256SUMS has no checksum for $AssetName."
    }

    $expected = ($line -split '\s+')[0]
    $actual = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash
    if ($actual -ne $expected) {
        throw "SHA-256 verification failed for $AssetName."
    }
}

function Confirm-AttestationsIfAvailable {
    param([string[]]$Paths)

    if (-not (Test-GitHubCliAuthentication)) {
        Write-Host 'NOTE: SHA-256 verified; install and authenticate GitHub CLI to also verify artifact provenance.'
        return
    }

    foreach ($path in $Paths) {
        & gh attestation verify $path --repo $repository | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "GitHub artifact attestation verification failed for $path with exit code $LASTEXITCODE."
        }
    }
}

function Test-GitHubCliAuthentication {
    if (-not (Get-Command 'gh' -ErrorAction SilentlyContinue)) {
        return $false
    }

    & gh auth status *> $null
    return $LASTEXITCODE -eq 0
}

function Backup-Installation {
    param([string]$BackupDirectory)

    New-Item -ItemType Directory -Path $BackupDirectory | Out-Null
    foreach ($path in @($binaryPath, $licensePath, $thirdPartyLicensesPath, $installerPath)) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            Copy-Item -LiteralPath $path -Destination $BackupDirectory
        }
    }

    $runCommand = Get-ItemPropertyValue -Path $runKey -Name $runValue -ErrorAction SilentlyContinue
    if ($null -ne $runCommand) {
        Set-Content -LiteralPath (Join-Path $BackupDirectory 'run-command.txt') -Value $runCommand
    }
}

function Stop-Overlay {
    $processes = @(Get-Process -Name 'keymap-overlay' -ErrorAction SilentlyContinue)
    if ($processes.Count -eq 0) {
        return
    }

    $processes | Stop-Process -Force
    try {
        $processes | Wait-Process -Timeout 10
    }
    catch {
        throw 'The running keymap-overlay process did not stop within 10 seconds.'
    }
}

function Install-StagedFiles {
    param([string]$TemporaryDirectory)

    New-Item -ItemType Directory -Path $assetDirectory -Force | Out-Null
    New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $TemporaryDirectory 'keymap-overlay.exe') -Destination $binaryPath -Force
    Copy-Item -LiteralPath (Join-Path $TemporaryDirectory 'LICENSE') -Destination $licensePath -Force
    Copy-Item -LiteralPath (Join-Path $TemporaryDirectory 'THIRD-PARTY-LICENSES.html') -Destination $thirdPartyLicensesPath -Force
    Copy-Item -LiteralPath (Join-Path $TemporaryDirectory 'release-install.ps1') -Destination $installerPath -Force
}

function Install-Autostart {
    $quotedBinary = '"{0}"' -f $binaryPath
    $quotedAssets = '"{0}"' -f $assetDirectory
    Set-ItemProperty -Path $runKey -Name $runValue -Value "$quotedBinary $quotedAssets"
    Start-Process -FilePath $binaryPath -ArgumentList $quotedAssets
}

function Restore-Installation {
    param([string]$BackupDirectory)

    foreach ($path in @($binaryPath, $licensePath, $thirdPartyLicensesPath, $installerPath)) {
        $backup = Join-Path $BackupDirectory (Split-Path -Leaf $path)
        if (Test-Path -LiteralPath $backup -PathType Leaf) {
            Copy-Item -LiteralPath $backup -Destination $path -Force
        }
        else {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
    }

    $runCommandPath = Join-Path $BackupDirectory 'run-command.txt'
    if (Test-Path -LiteralPath $runCommandPath -PathType Leaf) {
        Set-ItemProperty -Path $runKey -Name $runValue -Value (Get-Content -LiteralPath $runCommandPath -Raw).Trim()
    }
    else {
        Remove-ItemProperty -Path $runKey -Name $runValue -ErrorAction SilentlyContinue
    }
}

function Restart-PreviousInstallation {
    if (Test-Path -LiteralPath $binaryPath -PathType Leaf) {
        $quotedAssets = '"{0}"' -f $assetDirectory
        Start-Process -FilePath $binaryPath -ArgumentList $quotedAssets
    }
}

function Write-InstalledFiles {
    param([string]$ReleaseTag)

    Write-Output 'Installed:'
    Write-Output "  binary: $binaryPath"
    Write-Output "  license: $licensePath"
    Write-Output "  third-party licenses: $thirdPartyLicensesPath"
    Write-Output "  installer: $installerPath"
    Write-Output "  autostart: $runKey\$runValue"
    Write-Output "Using existing layer models: $assetDirectory"
    Write-Output "Logs: $logDirectory"
    Write-Output "Verified release: $ReleaseTag"
}

if ($MyInvocation.InvocationName -ne '.') {
    Main -Remove:$Uninstall
}
