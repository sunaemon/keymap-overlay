# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

BeforeAll {
    . (Join-Path $PSScriptRoot '..\windows.ps1') -Task build
}

Describe 'Windows development workflow' {
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
}
