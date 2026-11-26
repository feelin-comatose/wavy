"""Discord webhook: posts "Now playing" to a channel (example addon, standard library only).
Server settings → Integrations → Webhooks → New webhook → Copy URL, then paste it into config.json."""
import json, os, sys, time, urllib.request

API, TOKEN = os.environ["WAVY_API"], os.environ["WAVY_TOKEN"]
HERE = os.environ.get("WAVY_ADDON_DIR", ".")
cfg = json.load(open(os.path.join(HERE, "config.json"), encoding="utf-8"))
if not cfg.get("webhook", "").startswith("https://"):
    sys.exit("config.json: set \"webhook\" to your Discord webhook URL")


def events():
    while True:
        try:
            with urllib.request.urlopen(f"{API}/events?token={TOKEN}", timeout=None) as r:
                for line in r:
                    if line.startswith(b"data: "):
                        yield json.loads(line[6:])
        except OSError:
            time.sleep(3)


def post(text, thumb=None):
    body = {"content": text}
    if thumb and thumb.startswith("http"):
        body = {"embeds": [{"description": text, "thumbnail": {"url": thumb}}]}
    req = urllib.request.Request(cfg["webhook"], json.dumps(body).encode(), {"Content-Type": "application/json", "User-Agent": "Wavy addon"})
    try:
        urllib.request.urlopen(req, timeout=10).close()
    except OSError as e:
        print("webhook failed:", e, flush=True)


last = None
for ev in events():
    t = ev.get("track")
    if not t or not ev.get("playing") or t["id"] == last:
        continue
    if 0 < (ev.get("duration") or 0) < cfg.get("skip_shorter_than_sec", 30):
        continue
    last = t["id"]
    line = f"🎵 **{t.get('title', '')}** — {t.get('artist', '')}"
    if t.get("url"):
        line += f"\n{t['url']}"
    post(line, t.get("cover"))
