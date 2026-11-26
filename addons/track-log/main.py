"""Track log: appends each track Wavy starts playing to plays.csv (example addon, standard library only)."""
import csv, json, os, time, urllib.request

API, TOKEN = os.environ["WAVY_API"], os.environ["WAVY_TOKEN"]
OUT = os.path.join(os.environ.get("WAVY_ADDON_DIR", "."), "plays.csv")


def events():
    """Server-sent events from Wavy: one JSON object per change. Reconnects if Wavy restarts."""
    while True:
        try:
            with urllib.request.urlopen(f"{API}/events?token={TOKEN}", timeout=None) as r:
                for line in r:
                    if line.startswith(b"data: "):
                        yield json.loads(line[6:])
        except OSError:
            time.sleep(3)


last = None
for ev in events():
    t = ev.get("track")
    if not t or not ev.get("playing") or t["id"] == last:
        continue
    last = t["id"]
    new = not os.path.exists(OUT)
    with open(OUT, "a", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        if new:
            w.writerow(["time", "artist", "title", "url"])
        w.writerow([time.strftime("%Y-%m-%d %H:%M:%S"), t.get("artist", ""), t.get("title", ""), t.get("url") or ""])
    print("logged:", t.get("artist"), "-", t.get("title"), flush=True)
