# VS Code family adapter (Cursor / Windsurf / Trae / VSCodium / Code)

Optional enrichment. Layer 1 already attributes these editors via process
labels. This adapter appends allowlisted lines to `~/.aitime/agent_events.jsonl`
(or `$STRUGGLETIME_HOME/agent_events.jsonl`) — counts + labels only, never
document text or paths.

## Install

```bash
struggletime adapters install vscode
```

Then **Developer: Reload Window** in Cursor / VS Code.

## Heartbeat path

1. Adapter emits JSONL → `~/.aitime/agent_events.jsonl`
2. Local `struggletime` daemon drains that file into the project ledger
3. Daemon `POST`s sealed buckets to your server (`https://struggletime.screenseekr.com` by default)

Requirements for upstream sync:

- `struggletime apikey <st_…>`
- `struggletime link <repo>` / `struggletime start` (daemon running with a linked project)
- `struggletime server` pointing at `https://struggletime.screenseekr.com` (or your host)

## Verify live

1. Command Palette → **StruggleTime: Status** (opens Output → StruggleTime)
2. Edit/save a file; queued lines should increase
3. `struggletime status` → daemon running, API key set
4. Dashboard / `struggletime.screenseekr.com` after the next bucket seal (~5 min, or `struggletime sync`)

Debug: set `struggletime.debug` in settings, or `STRUGGLETIME_DEBUG=1` when launching the editor.
