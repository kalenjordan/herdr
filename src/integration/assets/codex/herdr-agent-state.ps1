# installed by herdr
# managed by herdr; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# HERDR_INTEGRATION_ID=codex
# HERDR_INTEGRATION_VERSION=7

param([string]$Action = "")

if ($Action -ne "session") { exit 0 }
if ($env:HERDR_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:HERDR_PANE_ID)) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    exit 0
}

if ($payload.hook_event_name -and $payload.hook_event_name -ne "SessionStart") { exit 0 }

$sessionId = $payload.session_id
if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

$cwd = if ($payload.cwd -is [string] -and $payload.cwd) { $payload.cwd } else { (Get-Location).Path }
try {
    $paneList = & herdr pane list 2>$null | ConvertFrom-Json
    $existing = @($paneList.result.panes | Where-Object {
        $_.agent_session.kind -eq "id" -and $_.agent_session.value -eq $sessionId
    })
    if ($existing.Count -eq 1) {
        $livePaneId = $existing[0].pane_id
    } else {
        $matches = @($paneList.result.panes | Where-Object {
            $_.agent -eq "codex" -and !$_.agent_session -and ($_.cwd -eq $cwd -or $_.foreground_cwd -eq $cwd)
        })
        if ($matches.Count -ne 1) { exit 0 }
        $livePaneId = $matches[0].pane_id
    }
} catch {
    exit 0
}

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
try {
    $args = @(
        "pane",
        "report-agent-session",
        $livePaneId,
        "--source",
        "herdr:codex",
        "--agent",
        "codex",
        "--seq",
        "$seq",
        "--agent-session-id",
        "$sessionId"
    )
    if ($payload.hook_event_name -eq "SessionStart" -and $payload.source -is [string] -and -not [string]::IsNullOrWhiteSpace($payload.source)) {
        $args += @("--session-start-source", "$($payload.source)")
    }
    & herdr @args 2>$null | Out-Null
} catch {
}
