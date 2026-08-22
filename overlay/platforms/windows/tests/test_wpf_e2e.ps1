$ErrorActionPreference = "Stop"

$projectDirectory = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..\..")).Path
$assetDirectory = Join-Path $projectDirectory "overlay\tests\fixtures"
$overlay = if ($env:KEYMAP_OVERLAY_E2E_OVERLAY) {
    $env:KEYMAP_OVERLAY_E2E_OVERLAY
} else {
    Join-Path $projectDirectory "target\wpf-publish\keymap-overlay.exe"
}
$testDirectory = Join-Path ([IO.Path]::GetTempPath()) ("keymap-overlay-e2e-" + [guid]::NewGuid())
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
try {
    $env:KEYMAP_OVERLAY_E2E_STATE_FILE = $stateFile
    $process = Start-Process -FilePath $overlay `
        -ArgumentList "--asset-dir", $assetDirectory, "--simulate", "1:2" `
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
