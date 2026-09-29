param(
    [ValidateSet('single', 'ticks', 'spinner', 'multi', 'normal', 'compact',
        'update-label', 'update-detail', 'update', 'update-multi', 'clean')]
    [string]$Scenario = 'multi',
    [ValidateRange(10, 3600)]
    [int]$DurationSeconds = 30
)

$ErrorActionPreference = 'Stop'

function Invoke-Bossbar {
    param([string[]]$CommandArgs)
    & bossbar @CommandArgs
    if ($LASTEXITCODE -ne 0) {
        throw "bossbar failed: $($CommandArgs -join ' ')"
    }
}

Get-Command bossbar -ErrorAction Stop | Out-Null
$stateJson = & bossbar list --json
if ($LASTEXITCODE -ne 0) { throw 'Could not read bossbar state' }
$previous = $stateJson | ConvertFrom-Json

if ($Scenario -eq 'clean') {
    foreach ($bar in $previous.bars) {
        if ($bar.id.StartsWith('bossbar-demo-')) {
            Invoke-Bossbar @('--no-spawn', 'remove', $bar.id)
        }
    }
    return
}

$prefix = 'bossbar-demo-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$ids = [Collections.Generic.List[string]]::new()

function New-DemoBar {
    param([string]$Name, [string]$Label, [string[]]$Options)
    $id = "$prefix-$Name"
    $ids.Add($id)
    Invoke-Bossbar (@('create', $Label, '--id', $id) + $Options)
}

try {
    Invoke-Bossbar @('visible', 'on')
    if ($Scenario -in @('normal', 'compact')) {
        Invoke-Bossbar @('collapse-mode', $Scenario)
        Invoke-Bossbar @('collapse', 'on')
    } else {
        Invoke-Bossbar @('collapse', 'off')
    }

    switch ($Scenario) {
        { $_ -in @('single', 'update-label', 'update-detail', 'update') } {
            New-DemoBar 'percent' 'Building wire-app' @(
                '--percent', '47', '--detail', 'cargo build --release - 214 crates', '--color', 'blue'
            )
        }
        'ticks' {
            New-DemoBar 'ticks' 'Compiling bossbar' @(
                '--ticks', '20', '--tick', '7', '--detail', '7 of 20 steps complete', '--color', 'cyan'
            )
        }
        'spinner' {
            New-DemoBar 'spinner' 'Waiting for device' @(
                '--indeterminate', '--detail', 'Connecting to the device', '--color', 'violet'
            )
        }
        default {
            New-DemoBar 'percent' 'Building wire-app' @(
                '--percent', '47', '--detail', 'cargo build --release - 214 crates', '--color', 'blue'
            )
            New-DemoBar 'ticks' 'Compiling bossbar' @(
                '--ticks', '20', '--tick', '7', '--detail', '7 of 20 steps complete', '--color', 'cyan'
            )
            New-DemoBar 'spinner' 'Waiting for device' @(
                '--indeterminate', '--detail', 'Connecting to the device', '--color', 'violet'
            )
        }
    }

    Write-Host "Demo '$Scenario' stays visible for $DurationSeconds seconds. Take a screenshot or press Ctrl+C to stop."
    if ($Scenario.StartsWith('update')) {
        Write-Host 'Text updates every 3 seconds; the first update follows a 3-second hold.'
        $labels = @('Building wire-app', 'Linking release binary', 'Packaging desktop app')
        $details = @('Compiling 214 crates', 'Linking optimized objects', 'Copying binaries into place')
        $tickLabels = @('Compiling bossbar', 'Optimizing bossbar', 'Packaging bossbar')
        $spinnerLabels = @('Waiting for device', 'Connecting to device', 'Negotiating connection')
        $spinnerDetails = @('Discovering nearby devices', 'Opening a connection', 'Waiting for the handshake')
        $percentages = @('35', '60', '85')
        $ticks = @('7', '12', '17')
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $nextUpdate = 3.0
        $step = 0
        while ($clock.Elapsed.TotalSeconds -lt $DurationSeconds) {
            Start-Sleep -Milliseconds 100
            if ($clock.Elapsed.TotalSeconds -ge $nextUpdate -and
                $clock.Elapsed.TotalSeconds -lt $DurationSeconds) {
                $step = ($step + 1) % $labels.Count
                foreach ($id in $ids) {
                    $patch = @('update', $id)
                    if ($Scenario -ne 'update-detail') {
                        $label = if ($id.EndsWith('-ticks')) { $tickLabels[$step] }
                            elseif ($id.EndsWith('-spinner')) { $spinnerLabels[$step] }
                            else { $labels[$step] }
                        $patch += @('--label', $label)
                    }
                    if ($Scenario -ne 'update-label') {
                        $detail = if ($id.EndsWith('-spinner')) { $spinnerDetails[$step] }
                            else { $details[$step] }
                        $patch += @('--detail', $detail)
                    }
                    if ($Scenario -in @('update', 'update-multi')) {
                        if ($id.EndsWith('-percent')) { $patch += @('--percent', $percentages[$step]) }
                        if ($id.EndsWith('-ticks')) { $patch += @('--tick', $ticks[$step]) }
                    }
                    Invoke-Bossbar $patch
                }
                $nextUpdate = $clock.Elapsed.TotalSeconds + 3.0
            }
        }
    } else {
        Start-Sleep -Seconds $DurationSeconds
    }
} finally {
    # Remove only this run's bars, then restore the previous display settings.
    foreach ($id in $ids) {
        & bossbar --no-spawn remove $id *> $null
    }
    & bossbar --no-spawn collapse-mode $previous.collapse_mode *> $null
    $collapsed = if ($previous.collapsed) { 'on' } else { 'off' }
    $visible = if ($previous.visible) { 'on' } else { 'off' }
    & bossbar --no-spawn collapse $collapsed *> $null
    & bossbar --no-spawn visible $visible *> $null
}
