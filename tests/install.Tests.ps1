BeforeAll {
    . (Join-Path $PSScriptRoot '..\install.ps')
}

Describe 'install.ps' {
    BeforeEach {
        $script:assetDirectory = Join-Path $TestDrive '.config\keymap-overlay'
        $script:binaryPath = Join-Path $assetDirectory 'keymap-overlay.exe'
        $script:licensePath = Join-Path $assetDirectory 'LICENSE'
        $script:thirdPartyLicensesPath = Join-Path $assetDirectory 'THIRD-PARTY-LICENSES.html'
        $script:installerPath = Join-Path $assetDirectory 'install.ps'
        $script:logDirectory = Join-Path $TestDrive '.local\var\log\keymap-overlay'
        New-Item -ItemType Directory -Path $assetDirectory -Force | Out-Null
    }

    It 'verifies a matching SHA-256 manifest entry' {
        $archive = Join-Path $TestDrive 'release.zip'
        $manifest = Join-Path $TestDrive 'SHA256SUMS'
        Set-Content -LiteralPath $archive -Value 'release fixture' -NoNewline
        $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        Set-Content -LiteralPath $manifest -Value "$hash  release.zip"

        { Confirm-Checksum -ArchivePath $archive -ChecksumsPath $manifest -AssetName 'release.zip' } | Should -Not -Throw
    }

    It 'rejects a mismatched SHA-256 manifest entry' {
        $archive = Join-Path $TestDrive 'release.zip'
        $manifest = Join-Path $TestDrive 'SHA256SUMS'
        Set-Content -LiteralPath $archive -Value 'release fixture' -NoNewline
        Set-Content -LiteralPath $manifest -Value "$('0' * 64)  release.zip"

        { Confirm-Checksum -ArchivePath $archive -ChecksumsPath $manifest -AssetName 'release.zip' } | Should -Throw
    }

    It 'does not require GitHub CLI for attestation verification' {
        Mock Get-Command { $null } -ParameterFilter { $Name -eq 'gh' }

        { Confirm-AttestationsIfAvailable -Paths @('release.zip', 'SHA256SUMS') } | Should -Not -Throw
    }

    It 'extracts and validates a complete release archive' {
        $fixture = Join-Path $TestDrive 'fixture'
        $archive = Join-Path $TestDrive 'fixture.zip'
        New-Item -ItemType Directory -Path $fixture | Out-Null
        Set-Content -LiteralPath (Join-Path $fixture 'keymap-overlay.exe') -Value 'binary'
        Set-Content -LiteralPath (Join-Path $fixture 'LICENSE') -Value 'license'
        Set-Content -LiteralPath (Join-Path $fixture 'THIRD-PARTY-LICENSES.html') -Value 'notices'
        Compress-Archive -Path (Join-Path $fixture '*') -DestinationPath $archive
        $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        $installerFixture = Join-Path $TestDrive 'installer-fixture.ps'
        Set-Content -LiteralPath $installerFixture -Value 'installer'
        $installerHash = (Get-FileHash -LiteralPath $installerFixture -Algorithm SHA256).Hash.ToLowerInvariant()

        Mock Invoke-RestMethod { [pscustomobject]@{ tag_name = 'v0.0.1' } }
        Mock Invoke-WebRequest {
            param([string]$Uri, [string]$OutFile)
            if ($Uri -like '*/SHA256SUMS') {
                Set-Content -LiteralPath $OutFile -Value @(
                    "$hash  keymap-overlay-windows-x86_64.zip"
                    "$installerHash  install.ps"
                )
            }
            elseif ($Uri -like '*/install.ps') {
                Copy-Item -LiteralPath $installerFixture -Destination $OutFile
            }
            else {
                Copy-Item -LiteralPath $archive -Destination $OutFile
            }
        }
        Mock Confirm-AttestationsIfAvailable

        $staging = Join-Path $TestDrive 'staging'
        New-Item -ItemType Directory -Path $staging | Out-Null
        Stage-Release -TemporaryDirectory $staging | Should -Be 'v0.0.1'
        (Join-Path $staging 'keymap-overlay.exe') | Should -Exist
        (Join-Path $staging 'LICENSE') | Should -Exist
        (Join-Path $staging 'THIRD-PARTY-LICENSES.html') | Should -Exist
        (Join-Path $staging 'release-install.ps') | Should -Exist
    }

    It 'keeps layer images and logs when uninstalling' {
        New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $assetDirectory '1_L0.png') -Value 'png'
        Set-Content -LiteralPath $binaryPath -Value 'binary'
        Set-Content -LiteralPath $licensePath -Value 'license'
        Set-Content -LiteralPath $thirdPartyLicensesPath -Value 'notices'
        Set-Content -LiteralPath $installerPath -Value 'installer'
        Mock Stop-Overlay
        Mock Remove-ItemProperty

        Uninstall-Release

        $binaryPath | Should -Not -Exist
        $licensePath | Should -Not -Exist
        $thirdPartyLicensesPath | Should -Not -Exist
        $installerPath | Should -Not -Exist
        (Join-Path $assetDirectory '1_L0.png') | Should -Exist
        $logDirectory | Should -Exist
    }

    It 'restores an existing installation when autostart setup fails' {
        Set-Content -LiteralPath (Join-Path $assetDirectory '1_L0.png') -Value 'png'
        Set-Content -LiteralPath $binaryPath -Value 'old binary'
        Set-Content -LiteralPath $licensePath -Value 'old license'
        Set-Content -LiteralPath $thirdPartyLicensesPath -Value 'old notices'
        Set-Content -LiteralPath $installerPath -Value 'old installer'
        $env:PROCESSOR_ARCHITECTURE = 'AMD64'

        Mock Stage-Release {
            param([string]$TemporaryDirectory)
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'keymap-overlay.exe') -Value 'new binary'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'LICENSE') -Value 'new license'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'THIRD-PARTY-LICENSES.html') -Value 'new notices'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'release-install.ps') -Value 'new installer'
            return 'v0.0.1'
        }
        Mock Get-ItemPropertyValue { $null }
        Mock Stop-Overlay
        Mock Install-Autostart { throw 'autostart failed' }
        Mock Remove-ItemProperty
        Mock Restart-PreviousInstallation

        { Install-Release } | Should -Throw
        Get-Content -LiteralPath $binaryPath | Should -Be 'old binary'
        Get-Content -LiteralPath $licensePath | Should -Be 'old license'
        Get-Content -LiteralPath $thirdPartyLicensesPath | Should -Be 'old notices'
        Get-Content -LiteralPath $installerPath | Should -Be 'old installer'
        Should -Invoke Restart-PreviousInstallation -Times 1
    }
}
