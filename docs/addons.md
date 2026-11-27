# Wavy addons

An addon is a separate program that talks to Wavy through a small local HTTP API. It can read what's playing, follow changes live, and control the player. Addons don't run inside Wavy, so they don't add to its memory use, and an addon that crashes can't take the player down with it. They can be written in any language.

## Installing an addon

1. In Wavy: **Settings → Addons → Open addons folder**. That's `%APPDATA%\wavy\addons` on Windows or `~/.local/share/wavy/addons` on Linux.
2. Copy the addon's folder there, so you have `addons/<addon>/addon.json`.
3. Press **Refresh list**, then tick the addon. Wavy starts it right away and every time Wavy launches, and stops it when Wavy quits.

Each addon writes its output to `addon.log` in its folder (the **log** link). If it crashes, Settings shows the exit code.

The examples in this repository (`addons/track-log`, `addons/discord-webhook`, `addons/esp32-display`) are Python scripts that use only the standard library. `esp32-display` also comes with an Arduino sketch for a small ESP32 screen with buttons; its `README.md` explains the hardware and setup. They need Python installed and available as `python`. On Windows, if only the `py` launcher is installed, change `"run"` to `["py", "main.py"]`.

## Writing an addon

A folder with an `addon.json` manifest:

```json
{
  "name": "Track log",
  "description": "Writes every track you play to plays.csv.",
  "version": "1.0",
  "author": "you",
  "run": ["python", "main.py"]
}
```

- `run` is the command and its arguments. The working directory is the addon's folder; if the first item is a file in that folder, it's run from there, otherwise it's looked up in `PATH` (`python`, `node`, `powershell`, …). A compiled `.exe` works too: `"run": ["my-addon.exe"]`.
- On Windows the process starts without a console window.

Wavy passes three environment variables:

| Variable | Example | |
|---|---|---|
| `WAVY_API` | `http://127.0.0.1:3737/api/v1` | API base address (the port is the overlay port from settings) |
| `WAVY_TOKEN` | 64 hex characters | secret token; send it with every request |
| `WAVY_ADDON_DIR` | `…/addons/track-log` | the addon's own folder (for config and output files) |

## The API

Every request needs the token, either as a header, `Authorization: Bearer <token>`, or as `?token=<token>` in the URL (for `EventSource`, which can't set headers). Without it the API answers `401`. The token is stored in `addons/api-token`. Scripts that Wavy doesn't start can read it from there, or copy it with **Settings → Addons → Copy API token**. It gives control over the player, so don't share it.

The server listens only on `127.0.0.1`, so other computers on the network can't reach it.

### `GET /api/v1/state`

The full player state, refreshed about three times a second:

```json
{
  "track": { "id": "yt_abc", "title": "…", "artist": "…", "album": "…", "type": "youtube",
             "duration": 214.0, "url": "https://…", "cover": "https://… or http://127.0.0.1:3737/cover/<id>" },
  "playing": true, "position": 63.2, "duration": 214.0, "liked": false,
  "volume": 0.8, "shuffle": false, "repeat": "off",
  "queueIndex": 4,
  "queue": [ { "index": 4, "id": "…", "title": "…", "artist": "…", "duration": 214.0 }, … ]
}
```

`track` is `null` when nothing is loaded. `queue` starts at the current track and holds up to 50 entries.

### `GET /api/v1/events`

A stream of [server-sent events](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events). It sends one `data: {json}` line right away, then another on every change: a new track, play/pause, a seek, or a queue change. Each message has `track`, `playing`, `position`, `duration`, `at` (the time of the update, in ms) and the overlay's `queue`/`lyrics`. Between messages, `position` advances in real time while `playing` is true. For anything else, call `/state`.

### `POST /api/v1/command`

JSON body `{"cmd": "...", ...}`. It answers `202 {"ok": true}` when the command is accepted.

| `cmd` | Extra fields | |
|---|---|---|
| `toggle` | | play/pause |
| `play`, `pause` | | |
| `next`, `prev` | | |
| `seek` | `value`: seconds | |
| `volume` | `value`: 0…1 | |
| `jump` | `value`: queue index | play that queue entry |
| `like` | `value`: true/false (optional) | like/unlike the current track; without `value` it toggles |
| `add` | `url` | import a link (YouTube, YouTube Music, Spotify, SoundCloud, …) into the library |

An unknown `cmd` gets `400` with the list of valid commands.

### `GET /api/v1/ping`

`{"app": "wavy", "api": 1}`: a quick way to check the address and token.

## Examples

```bash
TOKEN=$(cat ~/.local/share/wavy/addons/api-token)
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:3737/api/v1/state
curl -H "Authorization: Bearer $TOKEN" -d '{"cmd":"next"}' http://127.0.0.1:3737/api/v1/command
curl -N "http://127.0.0.1:3737/api/v1/events?token=$TOKEN"
```

PowerShell:

```powershell
$t = Get-Content "$env:APPDATA\wavy\addons\api-token"
Invoke-RestMethod -Headers @{Authorization="Bearer $t"} -Method Post -ContentType application/json -Body '{"cmd":"toggle"}' http://127.0.0.1:3737/api/v1/command
```

Python, following changes (the same loop the example addons use):

```python
import json, os, urllib.request
with urllib.request.urlopen(f"{os.environ['WAVY_API']}/events?token={os.environ['WAVY_TOKEN']}") as r:
    for line in r:
        if line.startswith(b"data: "):
            ev = json.loads(line[6:])
            print(ev["track"] and ev["track"]["title"], ev["playing"])
```

Ideas: a Stream Deck or macro-pad button (`curl` on a key), a Rainmeter or Waybar widget, pausing music when a call starts, your own OBS overlay, posting to a chat, or a sleep routine.
