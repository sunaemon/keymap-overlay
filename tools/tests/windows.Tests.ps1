# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

BeforeAll {
    . (Join-Path $PSScriptRoot '..\windows.ps1') -Task build
}

Describe 'Windows development workflow' {
    It 'runs the native overlay E2E path inside Rust coverage' {
        Mock Invoke-Mise
        Mock Invoke-MiseForOutput { 'set "LLVM_PROFILE_FILE=coverage.profraw"' }
        Mock Set-Content
        Mock Invoke-NativeCommand
        Mock Remove-Item

        Measure-RustCoverage

        Should -Invoke Invoke-Mise -Times 3
        Should -Invoke Invoke-MiseForOutput -Times 1 -ParameterFilter {
            $DevelopmentTools -and $Arguments -contains 'show-env' -and
            $Arguments -contains '--cmd'
        }
        Should -Invoke Invoke-NativeCommand -Times 1 -ParameterFilter {
            $Command -eq 'cmd.exe' -and $Arguments -contains '/c'
        }
    }

    It 'builds coverage commands for the instrumented native E2E binary' {
        $commands = New-WindowsCoverageCommands `
            'set "LLVM_PROFILE_FILE=coverage.profraw"' `
            'C:\coverage-target' `
            'C:\coverage-target\debug\keymap-overlay.exe' `
            'C:\src\test_win32_e2e.ps1'

        ($commands -join "`n") | Should -Match 'LLVM_PROFILE_FILE=coverage.profraw'
        ($commands -join "`n") | Should -Match 'cargo build --package keymap-overlay-windows'
        ($commands -join "`n") | Should -Match (
            [regex]::Escape('KEYMAP_OVERLAY_E2E_OVERLAY=C:\coverage-target')
        )
        ($commands -join "`n") | Should -Match 'test_win32_e2e.ps1'
    }

    It 'builds the release overlay before its acceptance E2E path' {
        Mock Build-Overlay
        Mock Invoke-WindowsOverlayE2e

        Test-WindowsOverlay

        Should -Invoke Build-Overlay -Times 1
        Should -Invoke Invoke-WindowsOverlayE2e -Times 1
    }

    It 'installs one native executable with startup refresh arguments' {
        Mock Build-Overlay
        Mock Stop-Overlay
        Mock New-Item
        Mock Copy-Item
        Mock Remove-Item
        Mock Remove-LegacyModels
        Mock Set-ItemProperty
        Mock Start-Process
        Mock Write-Output

        Install-Overlay

        Should -Invoke Copy-Item -Times 1 -ParameterFilter {
            $LiteralPath -eq $overlayBuildPath -and
            $Destination -eq $overlayInstallPath
        }
        Should -Invoke Set-ItemProperty -Times 1 -ParameterFilter {
            $Name -eq 'KeymapOverlay' -and
            $Value -like '*keymap-overlay.exe*--log-out*' -and
            $Value -notlike '*--asset-dir*' -and
            $Value -notlike '*--keyboard-config-dir*'
        }
        Should -Invoke Start-Process -Times 1 -ParameterFilter {
            $FilePath -eq $overlayInstallPath -and
            ($ArgumentList -join ' ') -notlike '*--asset-dir*' -and
            ($ArgumentList -join ' ') -notlike '*--keyboard-config-dir*'
        }
    }

    It 'stops the process before removing its executable and Run entry' {
        Mock Stop-Overlay
        Mock Remove-ItemProperty
        Mock Remove-Item
        Mock Remove-LegacyModels
        Mock Write-Output

        Uninstall-Overlay

        Should -Invoke Stop-Overlay -Times 1
        Should -Invoke Remove-ItemProperty -Times 1 -ParameterFilter {
            $Name -eq 'KeymapOverlay'
        }
        Should -Invoke Remove-Item -Times 1 -ParameterFilter {
            $LiteralPath -eq $overlayInstallPath
        }
    }

    It 'waits on the stopped process object when the overlay exits immediately' {
        $process = [System.Diagnostics.Process]::GetCurrentProcess()
        Mock Get-Process { $process }
        Mock Stop-Process { $InputObject }
        Mock Wait-Process

        Stop-Overlay

        Should -Invoke Stop-Process -Times 1 -ParameterFilter {
            $InputObject -eq $process -and $Force -and $PassThru
        }
        Should -Invoke Wait-Process -Times 1 -ParameterFilter {
            $InputObject -eq $process -and $Timeout -eq 10
        }
    }
}
