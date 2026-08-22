BeforeAll {
    . (Join-Path $PSScriptRoot '..\install.ps1')
}

Describe 'install.ps1' {
    BeforeEach {
        $script:assetDirectory = Join-Path $TestDrive 'AppData\Local\keymap-overlay'
        $script:programDirectory = Join-Path $TestDrive 'AppData\Local\Programs\keymap-overlay'
        $script:binaryPath = Join-Path $programDirectory 'keymap-overlay.exe'
        $script:generatorPath = Join-Path $programDirectory 'keymap-overlay-generator.exe'
        $script:generatorLicensesPath = Join-Path $programDirectory 'GENERATOR-THIRD-PARTY-LICENSES.html'
        $script:keyboardConfigDirectory = Join-Path $programDirectory 'keyboards'
        $script:licensePath = Join-Path $programDirectory 'LICENSE'
        $script:thirdPartyLicensesPath = Join-Path $programDirectory 'THIRD-PARTY-LICENSES.html'
        $script:installerPath = Join-Path $assetDirectory 'install.ps1'
        $script:logDirectory = Join-Path $TestDrive 'AppData\Local\keymap-overlay\logs'
        New-Item -ItemType Directory -Path $assetDirectory -Force | Out-Null
        New-Item -ItemType Directory -Path $programDirectory -Force | Out-Null
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

    It 'accepts the current Windows release architecture' {
        { Get-ReleaseArchitecture } | Should -Not -Throw
        Get-ReleaseArchitecture | Should -BeIn @('x86_64', 'arm64')
    }

    It 'extracts and validates a complete <Architecture> release archive' -ForEach @(
        @{ Architecture = 'arm64' }
        @{ Architecture = 'x86_64' }
    ) {
        $fixture = Join-Path $TestDrive "fixture-$Architecture"
        $archive = Join-Path $TestDrive "fixture-$Architecture.zip"
        New-Item -ItemType Directory -Path $fixture | Out-Null
        Set-Content -LiteralPath (Join-Path $fixture 'keymap-overlay.exe') -Value 'binary'
        Set-Content -LiteralPath (Join-Path $fixture 'keymap-overlay-generator.exe') -Value 'generator'
        Set-Content -LiteralPath (Join-Path $fixture 'GENERATOR-THIRD-PARTY-LICENSES.html') -Value 'generator notices'
        New-Item -ItemType Directory -Path (Join-Path $fixture 'keyboards\1') | Out-Null
        Set-Content -LiteralPath (Join-Path $fixture 'keyboards\1\config.json') -Value '{}'
        Set-Content -LiteralPath (Join-Path $fixture 'keyboards\1\keyboard.json') -Value '{}'
        Set-Content -LiteralPath (Join-Path $fixture 'LICENSE') -Value 'license'
        Set-Content -LiteralPath (Join-Path $fixture 'THIRD-PARTY-LICENSES.html') -Value 'notices'
        Compress-Archive -Path (Join-Path $fixture '*') -DestinationPath $archive
        $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        $installerFixture = Join-Path $TestDrive "installer-fixture-$Architecture.ps1"
        Set-Content -LiteralPath $installerFixture -Value 'installer'
        $installerHash = (Get-FileHash -LiteralPath $installerFixture -Algorithm SHA256).Hash.ToLowerInvariant()

        Mock Invoke-RestMethod { [pscustomobject]@{ tag_name = 'v0.0.1' } }
        $archiveName = "keymap-overlay-windows-$Architecture.zip"
        Mock Get-ReleaseArchitecture { $Architecture }
        Mock Invoke-WebRequest {
            param([string]$Uri, [string]$OutFile)
            if ($Uri -like '*/SHA256SUMS') {
                Set-Content -LiteralPath $OutFile -Value @(
                    "$hash  $archiveName"
                    "$installerHash  install.ps1"
                )
            }
            elseif ($Uri -like '*/install.ps1') {
                Copy-Item -LiteralPath $installerFixture -Destination $OutFile
            }
            else {
                Copy-Item -LiteralPath $archive -Destination $OutFile
            }
        }
        Mock Confirm-AttestationsIfAvailable

        $staging = Join-Path $TestDrive "staging-$Architecture"
        New-Item -ItemType Directory -Path $staging | Out-Null
        Stage-Release -TemporaryDirectory $staging | Should -Be 'v0.0.1'
        (Join-Path $staging 'keymap-overlay.exe') | Should -Exist
        (Join-Path $staging 'keymap-overlay-generator.exe') | Should -Exist
        (Join-Path $staging 'GENERATOR-THIRD-PARTY-LICENSES.html') | Should -Exist
        (Join-Path $staging 'keyboards\1\config.json') | Should -Exist
        (Join-Path $staging 'LICENSE') | Should -Exist
        (Join-Path $staging 'THIRD-PARTY-LICENSES.html') | Should -Exist
        (Join-Path $staging 'release-install.ps1') | Should -Exist
        Should -Invoke Invoke-WebRequest -Times 1 -ParameterFilter { $Uri -like "*/$archiveName" }
    }

    It 'keeps layer models and logs when uninstalling' {
        New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $assetDirectory '1.json') -Value '{}'
        Set-Content -LiteralPath $binaryPath -Value 'binary'
        Set-Content -LiteralPath $generatorPath -Value 'generator'
        Set-Content -LiteralPath $generatorLicensesPath -Value 'generator notices'
        New-Item -ItemType Directory -Path (Join-Path $keyboardConfigDirectory '1') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $keyboardConfigDirectory '1\config.json') -Value '{}'
        Set-Content -LiteralPath $licensePath -Value 'license'
        Set-Content -LiteralPath $thirdPartyLicensesPath -Value 'notices'
        Set-Content -LiteralPath $installerPath -Value 'installer'
        Mock Stop-Overlay
        Mock Remove-ItemProperty

        Uninstall-Release

        $binaryPath | Should -Not -Exist
        $generatorPath | Should -Not -Exist
        $generatorLicensesPath | Should -Not -Exist
        $keyboardConfigDirectory | Should -Not -Exist
        $licensePath | Should -Not -Exist
        $thirdPartyLicensesPath | Should -Not -Exist
        $installerPath | Should -Not -Exist
        (Join-Path $assetDirectory '1.json') | Should -Exist
        $logDirectory | Should -Exist
    }

    It 'forces the running overlay to stop and waits for it' {
        $process = [pscustomobject]@{ Id = 1234 }
        Mock Get-Process { @($process) } -ParameterFilter { $Name -eq 'keymap-overlay' }
        Mock Stop-Process
        Mock Wait-Process

        Stop-Overlay

        Should -Invoke Stop-Process -Times 1 -ParameterFilter { $Force }
        Should -Invoke Wait-Process -Times 1 -ParameterFilter { $Timeout -eq 10 }
    }

    It 'fails before replacement when the overlay remains running' {
        $process = [pscustomobject]@{ Id = 1234 }
        Mock Get-Process { @($process) } -ParameterFilter { $Name -eq 'keymap-overlay' }
        Mock Stop-Process
        Mock Wait-Process { throw 'timeout' }

        { Stop-Overlay } | Should -Throw '*did not stop within 10 seconds*'
    }

    It 'restores an existing installation when autostart setup fails' {
        Set-Content -LiteralPath (Join-Path $assetDirectory '1.json') -Value '{}'
        Set-Content -LiteralPath $binaryPath -Value 'old binary'
        Set-Content -LiteralPath $generatorPath -Value 'old generator'
        Set-Content -LiteralPath $generatorLicensesPath -Value 'old generator notices'
        New-Item -ItemType Directory -Path (Join-Path $keyboardConfigDirectory '1') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $keyboardConfigDirectory '1\config.json') -Value 'old config'
        Set-Content -LiteralPath $licensePath -Value 'old license'
        Set-Content -LiteralPath $thirdPartyLicensesPath -Value 'old notices'
        Set-Content -LiteralPath $installerPath -Value 'old installer'
        $env:PROCESSOR_ARCHITECTURE = 'AMD64'

        Mock Stage-Release {
            param([string]$TemporaryDirectory)
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'keymap-overlay.exe') -Value 'new binary'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'keymap-overlay-generator.exe') -Value 'new generator'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'GENERATOR-THIRD-PARTY-LICENSES.html') -Value 'new generator notices'
            New-Item -ItemType Directory -Path (Join-Path $TemporaryDirectory 'keyboards\1') | Out-Null
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'keyboards\1\config.json') -Value '{}'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'keyboards\1\keyboard.json') -Value '{}'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'LICENSE') -Value 'new license'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'THIRD-PARTY-LICENSES.html') -Value 'new notices'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'release-install.ps1') -Value 'new installer'
            return 'v0.0.1'
        }
        Mock Get-ItemPropertyValue { $null }
        Mock Stop-Overlay
        Mock Install-Autostart { throw 'autostart failed' }
        Mock Remove-ItemProperty
        Mock Restart-PreviousInstallation

        { Install-Release } | Should -Throw
        Get-Content -LiteralPath $binaryPath | Should -Be 'old binary'
        Get-Content -LiteralPath $generatorPath | Should -Be 'old generator'
        Get-Content -LiteralPath $generatorLicensesPath | Should -Be 'old generator notices'
        Get-Content -LiteralPath (Join-Path $keyboardConfigDirectory '1\config.json') | Should -Be 'old config'
        Get-Content -LiteralPath $licensePath | Should -Be 'old license'
        Get-Content -LiteralPath $thirdPartyLicensesPath | Should -Be 'old notices'
        Get-Content -LiteralPath $installerPath | Should -Be 'old installer'
        $logDirectory | Should -Exist
        Should -Invoke Restart-PreviousInstallation -Times 1
    }

    It 'continues rollback when stopping the failed installation times out' {
        Set-Content -LiteralPath (Join-Path $assetDirectory '1.json') -Value '{}'
        Set-Content -LiteralPath $binaryPath -Value 'old binary'
        Set-Content -LiteralPath $generatorPath -Value 'old generator'
        Set-Content -LiteralPath $generatorLicensesPath -Value 'old generator notices'
        New-Item -ItemType Directory -Path (Join-Path $keyboardConfigDirectory '1') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $keyboardConfigDirectory '1\config.json') -Value 'old config'
        Set-Content -LiteralPath $licensePath -Value 'old license'
        Set-Content -LiteralPath $thirdPartyLicensesPath -Value 'old notices'
        Set-Content -LiteralPath $installerPath -Value 'old installer'
        $env:PROCESSOR_ARCHITECTURE = 'AMD64'
        $script:stopCalls = 0

        Mock Stage-Release {
            param([string]$TemporaryDirectory)
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'keymap-overlay.exe') -Value 'new binary'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'keymap-overlay-generator.exe') -Value 'new generator'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'GENERATOR-THIRD-PARTY-LICENSES.html') -Value 'new generator notices'
            New-Item -ItemType Directory -Path (Join-Path $TemporaryDirectory 'keyboards\1') | Out-Null
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'keyboards\1\config.json') -Value '{}'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'keyboards\1\keyboard.json') -Value '{}'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'LICENSE') -Value 'new license'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'THIRD-PARTY-LICENSES.html') -Value 'new notices'
            Set-Content -LiteralPath (Join-Path $TemporaryDirectory 'release-install.ps1') -Value 'new installer'
            return 'v0.0.1'
        }
        Mock Get-ItemPropertyValue { $null }
        Mock Stop-Overlay {
            $script:stopCalls++
            if ($script:stopCalls -eq 2) { throw 'stop timeout' }
        }
        Mock Install-Autostart { throw 'autostart failed' }
        Mock Remove-ItemProperty
        Mock Restart-PreviousInstallation

        { Install-Release } | Should -Throw '*stopping the overlay: stop timeout*'
        Get-Content -LiteralPath $binaryPath | Should -Be 'old binary'
        Get-Content -LiteralPath $generatorPath | Should -Be 'old generator'
        Get-Content -LiteralPath $generatorLicensesPath | Should -Be 'old generator notices'
        Get-Content -LiteralPath (Join-Path $keyboardConfigDirectory '1\config.json') | Should -Be 'old config'
        Get-Content -LiteralPath $licensePath | Should -Be 'old license'
        Get-Content -LiteralPath $thirdPartyLicensesPath | Should -Be 'old notices'
        Get-Content -LiteralPath $installerPath | Should -Be 'old installer'
        Should -Invoke Restart-PreviousInstallation -Times 1
    }

}
