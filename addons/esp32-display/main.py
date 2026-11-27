"""ESP32 display: a bridge between Wavy and a small ESP32 gadget on your home Wi-Fi (standard library only).

Wavy's API only listens on 127.0.0.1, so the ESP32 can't reach it directly. This addon runs on the PC,
follows Wavy and relays a small part of it over the local network:
  - it announces itself with a UDP broadcast every 2 s, so the ESP32 finds the PC without an IP address;
  - the ESP32 connects over TCP, sends "key <key>", then gets one line per change and sends commands.

PC → ESP32, one line each, fields separated by tabs:
  S <playing 0/1> <liked 0/1> <volume 0-100> <position s> <duration s> <title> <artist>
  N                                      (nothing loaded)
ESP32 → PC: toggle | next | prev | like | vol+ | vol-

The key is generated on the first start and saved into config.json; copy it into the sketch."""
import json, os, secrets, socket, threading, time, unicodedata, urllib.request

API, TOKEN = os.environ["WAVY_API"], os.environ["WAVY_TOKEN"]
HERE = os.environ.get("WAVY_ADDON_DIR", ".")
CFG_PATH = os.path.join(HERE, "config.json")
cfg = json.load(open(CFG_PATH, encoding="utf-8"))
if not cfg.get("key"):
    cfg["key"] = secrets.token_hex(4)
    with open(CFG_PATH, "w", encoding="utf-8") as f:
        json.dump(cfg, f, indent=2)
KEY = cfg["key"]
PORT = int(cfg.get("port", 3739))
DISCOVERY_PORT = int(cfg.get("discovery_port", 3740))

clients, clients_lock = set(), threading.Lock()
dirty = threading.Event()
current = "N\n"


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def api_get(path):
    req = urllib.request.Request(f"{API}{path}", headers={"Authorization": f"Bearer {TOKEN}"})
    with urllib.request.urlopen(req, timeout=5) as r:
        return json.loads(r.read())


def api_cmd(body):
    req = urllib.request.Request(f"{API}/command", json.dumps(body).encode(),
                                 {"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"})
    try:
        urllib.request.urlopen(req, timeout=5).close()
    except OSError as e:
        log("command failed:", e)


def clean(s, limit=120):
    """Text the display fonts can draw: ASCII and Cyrillic. Accents are dropped (é → e), other scripts removed."""
    out = []
    for ch in str(s or ""):
        if ch in "\t\r\n":
            ch = " "
        if 32 <= ord(ch) < 127 or 0x400 <= ord(ch) <= 0x45F:
            out.append(ch)
        else:
            out.extend(c for c in unicodedata.normalize("NFKD", ch) if 32 <= ord(c) < 127)
    return " ".join("".join(out).split())[:limit]


def line_from(st):
    t = st.get("track")
    if not t:
        return "N\n"
    title = clean(t.get("title")) or "?"
    fields = ["S", int(bool(st.get("playing"))), int(bool(st.get("liked"))),
              round((st.get("volume") or 0) * 100), round(st.get("position") or 0, 1),
              round(st.get("duration") or t.get("duration") or 0), title, clean(t.get("artist"))]
    return "\t".join(map(str, fields)) + "\n"


def queue_from(st):
    """The next three tracks: "Q", then index, title and artist for each (older sketches ignore this line)."""
    fields = ["Q"]
    if st.get("track"):
        for q in (st.get("queue") or [])[1:4]:
            fields += [q.get("index", 0), clean(q.get("title"), 80) or "?", clean(q.get("artist"), 60)]
    return "\t".join(map(str, fields)) + "\n"


def send_all(line):
    with clients_lock:
        for c in list(clients):
            try:
                c.sendall(line.encode("utf-8"))
            except OSError:
                clients.discard(c)
                c.close()


def follow_events():
    """Server-sent events from Wavy: wake the main loop on every change. Reconnects if Wavy restarts."""
    while True:
        try:
            with urllib.request.urlopen(f"{API}/events?token={TOKEN}", timeout=None) as r:
                for line in r:
                    if line.startswith(b"data: "):
                        dirty.set()
        except OSError:
            time.sleep(3)


def command(cmd):
    if cmd in ("toggle", "next", "prev", "like"):
        api_cmd({"cmd": cmd})
    elif cmd in ("vol+", "vol-"):
        try:
            vol = api_get("/state").get("volume") or 0
        except OSError:
            return
        vol = min(1.0, max(0.0, vol + (0.05 if cmd == "vol+" else -0.05)))
        api_cmd({"cmd": "volume", "value": round(vol, 2)})
    elif cmd.startswith("jump ") and cmd[5:].isdigit():
        api_cmd({"cmd": "jump", "value": int(cmd[5:])})
    else:
        return
    # /state refreshes about three times a second; a like or volume change has no event of its own.
    threading.Timer(0.4, dirty.set).start()


def serve_client(conn, addr):
    conn.settimeout(10)
    try:
        f = conn.makefile("r", encoding="utf-8", errors="replace")
        if f.readline().strip() != f"key {KEY}":
            log(f"{addr[0]}: wrong key, check KEY in the sketch")
            conn.sendall(b"E\tbad key\n")
            return
        log(f"{addr[0]}: connected")
        conn.settimeout(None)
        conn.sendall(current.encode("utf-8"))
        with clients_lock:
            clients.add(conn)
        for line in f:
            command(line.strip())
    except OSError:
        pass
    finally:
        with clients_lock:
            clients.discard(conn)
        conn.close()
        log(f"{addr[0]}: disconnected")


def listen():
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("0.0.0.0", PORT))
    srv.listen()
    while True:
        conn, addr = srv.accept()
        conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        threading.Thread(target=serve_client, args=(conn, addr), daemon=True).start()


def local_ip():
    """The address of the interface that leads to the network (connecting a UDP socket sends nothing)."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("10.255.255.255", 1))
        return s.getsockname()[0]
    except OSError:
        return None
    finally:
        s.close()


def announce():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    msg = f"WAVY-ESP32 {PORT}".encode()
    while True:
        targets = {"255.255.255.255"}
        ip = local_ip()
        if ip and not ip.startswith("127."):
            targets.add(ip.rsplit(".", 1)[0] + ".255")  # most home networks are /24
        for t in targets:
            try:
                s.sendto(msg, (t, DISCOVERY_PORT))
            except OSError:
                pass
        time.sleep(2)


threading.Thread(target=follow_events, daemon=True).start()
threading.Thread(target=listen, daemon=True).start()
threading.Thread(target=announce, daemon=True).start()
log(f"listening on port {PORT}, this PC is {local_ip() or '?'}; key for the sketch: {KEY}")

while True:
    dirty.wait(5)
    dirty.clear()
    try:
        st = api_get("/state")
        current = line_from(st) + queue_from(st)
    except OSError:
        time.sleep(1)
        continue
    send_all(current)  # also every 5 s, so the ESP32 knows the link is alive and corrects its clock
