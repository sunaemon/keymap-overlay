# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet(
        'setup',
        'format',
        'lint',
        'test',
        'test-rust',
        'coverage-rust',
        'build',
        'run',
        'test-installer',
        'test-windows-e2e',
        'test-release-acceptance',
        'install',
        'uninstall',
        'check-licenses',
        'check-commit-message'
    )]
    [string]$Task,
    [string]$Simulate,
    [string]$CommitMessageFile
)

$ErrorActionPreference = 'Stop'
$script:projectDirectory = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$script:overlayBuildPath = Join-Path $projectDirectory 'target\release\keymap-overlay.exe'
$script:overlayInstallDirectory = Join-Path $env:LOCALAPPDATA 'Programs\keymap-overlay'
$script:overlayInstallPath = Join-Path $overlayInstallDirectory 'keymap-overlay.exe'
$script:overlayLogDirectory = Join-Path $env:LOCALAPPDATA 'keymap-overlay\logs'
$script:overlayLogPath = Join-Path $overlayLogDirectory 'overlay.log'
$script:runKeyPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$script:runValueName = 'KeymapOverlay'

function Invoke-WindowsWorkflow {
    Set-Location $projectDirectory
    switch ($Task) {
        'setup' { Install-DevelopmentTools }
        'format' { Format-Repository }
        'lint' { Test-RepositoryStyle }
        'test' { Invoke-Mise @('exec', '--', 'uv', 'run', 'pytest') }
        'test-rust' { Test-Rust }
        'coverage-rust' { Measure-RustCoverage }
        'build' { Build-Overlay }
        'run' { Start-DevelopmentOverlay }
        'test-installer' { Test-Installer }
        'test-windows-e2e' { Test-WindowsOverlay }
        'test-release-acceptance' {
            Test-Installer
            Test-WindowsOverlay
        }
        'install' { Install-Overlay }
        'uninstall' { Uninstall-Overlay }
        'check-licenses' {
            Invoke-Mise -DevelopmentTools @(
                'exec', '--', 'uv', 'run', 'python', '-m',
                'installer.release.generate_license_report', '--check'
            )
        }
        'check-commit-message' {
            if ([string]::IsNullOrWhiteSpace($CommitMessageFile)) {
                throw '-CommitMessageFile is required for check-commit-message'
            }
            Invoke-Mise @(
                'exec', '--', 'uv', 'run', 'python',
                'tools/check_commit_message.py', $CommitMessageFile
            )
        }
    }
}

function Install-DevelopmentTools {
    Invoke-NativeCommand 'mise' @('trust')
    Invoke-NativeCommand 'mise' @('install', 'rust', 'python', 'uv', 'lefthook')
    Invoke-Mise -DevelopmentTools @('install')
    Invoke-Mise @('exec', '--', 'uv', 'sync')
    Invoke-Mise @('exec', '--', 'lefthook', 'install')
}

function Format-Repository {
    $previousEnvironment = $env:MISE_ENV
    $env:MISE_ENV = 'dev'
    try {
        Invoke-Mise @('exec', '--', 'uv', 'sync')
        Invoke-Mise @('exec', '--', 'ruff', 'format', '.')
        Invoke-Mise @('exec', '--', 'cargo', 'fmt')
        Invoke-ForGitFiles @('Makefile', '*.mk') 'mbake' @('format', '--config', '.mbake.toml')
        Invoke-ForGitFiles @('*.md', '*.yml', '*.yaml', '*.json', '*.js', '*.css') 'prettier' @('--write') -ExcludeLinks
        Invoke-ForGitFiles @('*.toml') 'taplo' @('fmt')
        Invoke-ForGitFiles @('*.c', '*.h', '*.cpp') 'clang-format' @('-i')
    }
    finally {
        $env:MISE_ENV = $previousEnvironment
    }
}

function Test-RepositoryStyle {
    Invoke-Mise -DevelopmentTools @('exec', '--', 'uv', 'sync')
    Invoke-Mise -DevelopmentTools @('exec', '--', 'ruff', 'check', '--fix', '.')
    # ty resolves POSIX-only os members against the host, while ShellCheck sees
    # Git's CRLF checkout. Linux CI owns those two repository-wide checks.
    Invoke-Mise -DevelopmentTools @(
        'exec', '--', 'cargo', 'clippy', '--workspace', '--all-targets',
        '--', '-D', 'warnings'
    )
}

function Test-Rust {
    Confirm-DisplayModelContract
    Invoke-Mise @(
        'exec', '--', 'cargo', 'test', '--workspace',
        '--exclude', 'keymap-overlay-winui'
    )
}

function Confirm-DisplayModelContract {
    $generated = Invoke-MiseForOutput @(
        'exec', '--', 'cargo', 'run', '--quiet',
        '--package', 'keymap-overlay-generator',
        '--features', 'contract-schema',
        '--bin', 'generate-display-model-schema'
    )
    $checkedPath = Join-Path $projectDirectory 'overlay\display-model-contract\display-model.schema.json'
    $checked = Get-Content -Raw -LiteralPath $checkedPath
    if ((Normalize-Newlines $generated) -cne (Normalize-Newlines $checked)) {
        throw "Generated display-model schema differs from $checkedPath"
    }
    Invoke-Mise @(
        'exec', '--', 'cargo', 'test',
        '--package', 'keymap-overlay-generator',
        '--features', 'contract-schema', 'contract::'
    )
}

function Measure-RustCoverage {
    Invoke-Mise -DevelopmentTools @(
        'exec', '--', 'cargo', 'llvm-cov', '--workspace',
        '--exclude', 'keymap-overlay-winui', '--all-targets', '--no-report'
    )
    Invoke-Mise -DevelopmentTools @(
        'exec', '--', 'cargo', 'llvm-cov', 'report',
        '--lcov', '--output-path', 'coverage-rust.lcov'
    )
    Invoke-Mise -DevelopmentTools @(
        'exec', '--', 'cargo', 'llvm-cov', 'report', '--summary-only'
    )
}

function Build-Overlay {
    Invoke-Mise @(
        'exec', '--', 'cargo', 'build', '--release',
        '--package', 'keymap-overlay-windows'
    )
}

function Start-DevelopmentOverlay {
    $arguments = @('exec', '--', 'cargo', 'run', '--package', 'keymap-overlay-windows', '--')
    if (-not [string]::IsNullOrWhiteSpace($Simulate)) {
        $arguments += @('--simulate', $Simulate)
    }
    Invoke-Mise $arguments
}

function Test-Installer {
    Invoke-NativeCommand 'powershell.exe' @(
        '-NoProfile', '-Command',
        'Invoke-Pester -Path @(''installer/tests/install.Tests.ps1'', ''tools/tests/windows.Tests.ps1'') -CI'
    )
}

function Test-WindowsOverlay {
    Build-Overlay
    Invoke-NativeCommand 'powershell.exe' @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
        'overlay/platforms/windows/tests/test_win32_e2e.ps1'
    )
}

function Install-Overlay {
    Build-Overlay
    Stop-Overlay
    New-Item -ItemType Directory -Path $overlayInstallDirectory -Force | Out-Null
    New-Item -ItemType Directory -Path $overlayLogDirectory -Force | Out-Null
    Copy-Item -LiteralPath $overlayBuildPath -Destination $overlayInstallPath -Force
    $legacyGenerator = Join-Path $overlayInstallDirectory 'keymap-overlay-generator.exe'
    Remove-Item -LiteralPath $legacyGenerator -Force -ErrorAction SilentlyContinue
    Remove-LegacyModels
    $command = "`"$overlayInstallPath`" --log-out `"$overlayLogPath`""
    Set-ItemProperty -Path $runKeyPath -Name $runValueName -Value $command
    Start-Process -FilePath $overlayInstallPath -ArgumentList @('--log-out', $overlayLogPath)
    Write-Output "Overlay installed and started; logs: $overlayLogDirectory"
}

function Uninstall-Overlay {
    Stop-Overlay
    Remove-ItemProperty -Path $runKeyPath -Name $runValueName -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $overlayInstallPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Join-Path $overlayInstallDirectory 'keymap-overlay-generator.exe') -Force -ErrorAction SilentlyContinue
    Remove-LegacyModels
    Write-Output "Overlay removed; logs remain at $overlayLogDirectory"
}

function Stop-Overlay {
    $processes = @(Get-Process -Name 'keymap-overlay' -ErrorAction SilentlyContinue)
    foreach ($process in $processes) {
        Stop-Process -Id $process.Id -Force
        Wait-Process -Id $process.Id -Timeout 10
    }
}

function Remove-LegacyModels {
    $assetDirectory = Join-Path $env:LOCALAPPDATA 'keymap-overlay'
    Get-ChildItem -LiteralPath $assetDirectory -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Extension -eq '.png' -or $_.Name -match '^\d+\.json$' } |
        Remove-Item -Force
}

function Invoke-ForGitFiles {
    param(
        [string[]]$Patterns,
        [string]$Command,
        [string[]]$Arguments,
        [switch]$ExcludeLinks
    )
    $files = @(& git ls-files -- @Patterns)
    Confirm-LastExitCode 'git ls-files'
    if ($ExcludeLinks) {
        $links = @(& git ls-files --stage -- @Patterns |
            Where-Object { $_ -match '^120000 ' } |
            ForEach-Object { ($_ -split "`t", 2)[1] })
        Confirm-LastExitCode 'git ls-files --stage'
        $files = @($files | Where-Object { $_ -notin $links })
    }
    if ($files.Count -gt 0) {
        Invoke-Mise -DevelopmentTools (@('exec', '--', $Command) + $Arguments + $files)
    }
}

function Invoke-Mise {
    param(
        [Parameter(Position = 0)]
        [string[]]$Arguments,
        [switch]$DevelopmentTools
    )
    $previousEnvironment = $env:MISE_ENV
    if ($DevelopmentTools) {
        $env:MISE_ENV = 'dev'
    }
    try {
        Invoke-NativeCommand 'mise' $Arguments
    }
    finally {
        $env:MISE_ENV = $previousEnvironment
    }
}

function Invoke-MiseForOutput {
    param([Parameter(Position = 0)][string[]]$Arguments)
    $output = & mise @Arguments | Out-String
    Confirm-LastExitCode 'mise'
    return $output
}

function Invoke-NativeCommand {
    param([string]$Command, [string[]]$Arguments)
    & $Command @Arguments
    Confirm-LastExitCode $Command
}

function Confirm-LastExitCode {
    param([string]$Command)
    if ($LASTEXITCODE -ne 0) {
        throw "$Command exited with code $LASTEXITCODE"
    }
}

function Normalize-Newlines {
    param([string]$Value)
    return ($Value -replace "`r`n", "`n").TrimEnd("`n")
}

if ($MyInvocation.InvocationName -ne '.') {
    Invoke-WindowsWorkflow
}
