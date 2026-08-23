$ErrorActionPreference = "Stop"

$projectDirectory = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..\..")).Path
$fixtureDirectory = Join-Path $projectDirectory "overlay\tests\fixtures"
$overlay = if ($env:KEYMAP_OVERLAY_E2E_OVERLAY) {
    $env:KEYMAP_OVERLAY_E2E_OVERLAY
} else {
    Join-Path $projectDirectory "target\wpf-publish\keymap-overlay.exe"
}
$testDirectory = Join-Path ([IO.Path]::GetTempPath()) ("keymap-overlay-e2e-" + [guid]::NewGuid())
$keyboardConfigDirectory = Join-Path $testDirectory "keyboards"
$assetDirectory = Join-Path $testDirectory "assets"
$stateFile = Join-Path $testDirectory "state"
$outputFile = Join-Path $testDirectory "overlay.out.log"
$errorFile = Join-Path $testDirectory "overlay.err.log"
$process = $null

function Fail-Test([string]$message) {
    Write-Error "WPF E2E failure: $message"
}

function Wait-ForState([string]$description, [string]$pattern, [int]$count = 1) {
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        if ($process.HasExited) {
            Fail-Test "overlay exited while waiting for $description"
        }
        $matches = if (Test-Path $stateFile) {
            @(Select-String -Path $stateFile -SimpleMatch $pattern).Count
        } else {
            0
        }
        if ($matches -ge $count) {
            return
        }
        Start-Sleep -Milliseconds 50
    }
    Fail-Test "timed out waiting for $description"
}

New-Item -ItemType Directory -Path $testDirectory | Out-Null
New-Item -ItemType Directory -Path $keyboardConfigDirectory | Out-Null
New-Item -ItemType Directory -Path $assetDirectory | Out-Null
Copy-Item -Path (Join-Path $fixtureDirectory "*.json") -Destination $assetDirectory
try {
    $legacyGenerator = Join-Path (Split-Path -Parent $overlay) "keymap-overlay-generator.exe"
    if (Test-Path -LiteralPath $legacyGenerator -PathType Leaf) {
        Fail-Test "legacy model generator must not be installed beside the WPF executable"
    }
    $env:KEYMAP_OVERLAY_E2E_STATE_FILE = $stateFile

    @'
{"keyboard_id":2,"layers":{"0":{"version":2,"layer":0,"width":160,"height":120,"header_font_size":14,"key_font_size":12,"encoder_font_size":10,"keys":null,"encoders":[]}}}
'@ | Set-Content -LiteralPath (Join-Path $assetDirectory "2.json")
    $process = Start-Process -FilePath $overlay `
        -ArgumentList "--asset-dir", $assetDirectory, `
            "--keyboard-config-dir", $keyboardConfigDirectory, "--simulate", "2:0" `
        -RedirectStandardOutput $outputFile -RedirectStandardError $errorFile -PassThru
    Wait-ForState "a malformed model to be ignored" "hide size=1x1"
    if ($process.HasExited) {
        Fail-Test "overlay exited while rejecting a malformed model"
    }
    Stop-Process -Id $process.Id -Force
    $process.WaitForExit()
    $process = $null
    Remove-Item -LiteralPath $stateFile -Force

    $process = Start-Process -FilePath $overlay `
        -ArgumentList "--asset-dir", $assetDirectory, `
            "--keyboard-config-dir", $keyboardConfigDirectory, "--simulate", "1:2" `
        -RedirectStandardOutput $outputFile -RedirectStandardError $errorFile -PassThru

    Wait-ForState "the composed layer to be attached" `
        "show keyboard=1 layers=[2] size=162x122 keys=2 encoders=0 held=1"
    Wait-ForState "the simulated release to detach and hide the layer" "hide size=1x1"
    Wait-ForState "the next simulated press to attach the layer again" `
        "show keyboard=1 layers=[2] size=162x122 keys=2 encoders=0 held=1" 2

    if ($process.HasExited) {
        Fail-Test "overlay exited while processing WPF state transitions"
    }
    Write-Output "Windows WPF E2E test passed"
} catch {
    if (Test-Path $outputFile) {
        Write-Warning "Overlay output:`n$((Get-Content -Raw $outputFile))"
    }
    if (Test-Path $errorFile) {
        Write-Warning "Overlay errors:`n$((Get-Content -Raw $errorFile))"
    }
    if (Test-Path $stateFile) {
        Write-Error "Observed WPF states:`n$((Get-Content -Raw $stateFile))"
    }
    throw
} finally {
    Remove-Item Env:KEYMAP_OVERLAY_E2E_STATE_FILE -ErrorAction SilentlyContinue
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
    }
    Remove-Item -LiteralPath $testDirectory -Recurse -Force -ErrorAction SilentlyContinue
}
