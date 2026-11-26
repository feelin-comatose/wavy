#!/usr/bin/env python3
"""Measure Wavy's own memory on Linux while it does real work.

Runs the built app with a generated demo library (local files, so no network is needed), drives it
through the addon API and xdotool, and samples /proc/<pid>/smaps_rollup in each phase:
  idle → scrolling a large library with covers → playing music → playing a video → music again.

"Own memory" is private dirty memory (heap, buffers, textures: what Wavy itself allocated and wrote),
the closest match to the "Memory" column in Windows Task Manager (private working set). USS also counts
file-backed code pages (Wavy's own binary, libraries) that only Wavy happens to have loaded; Task
Manager doesn't count those, so USS is shown for reference only.

Caveat: on a machine without a GPU (CI, containers) Mesa renders in software inside the process, so
its buffers count as Wavy's memory and the numbers come out higher than on a real PC. The report
says which renderer was used.

Usage:
  cargo build --release -p wavy-native
  python3 scripts/mem-check.py [path/to/wavy-native] [--limit-mb 100] [--tracks 500] [--phase-sec 12] [--cycles N]
  --cycles 4 repeats video → music: memory that keeps growing each cycle is a leak, a flat level is the
  allocator keeping freed memory for reuse. WAVY_MEM_DEBUG=1 also prints the biggest private mappings.
Needs: python3 (standard library only), ffmpeg (with libx264), xdotool, Xvfb if there's no $DISPLAY, pulseaudio (optional:
without it audio goes to a null ALSA device, which plays faster than real time).
Exit code 1 if peak private memory while playing music is over --limit-mb (CLAUDE.md: up to ~100 MB is fine).
"""
import argparse, json, os, shutil, signal, socket, subprocess, sys, tempfile, time, urllib.request

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("binary", nargs="?", default="target/release/wavy-native")
ap.add_argument("--limit-mb", type=float, default=100.0, help="max private memory while playing music (default 100)")
ap.add_argument("--tracks", type=int, default=500, help="library size (default 500)")
ap.add_argument("--phase-sec", type=float, default=12.0, help="how long each phase is sampled (default 12)")
ap.add_argument("--cycles", type=int, default=1, help="repeat video → music this many times (growth each time = a leak)")
ap.add_argument("--keep", action="store_true", help="keep the temporary folder (for debugging)")
args = ap.parse_args()

if not sys.platform.startswith("linux"):
    sys.exit("mem-check works on Linux only (it reads /proc)")
binary = os.path.abspath(args.binary)
if not os.path.exists(binary):
    sys.exit(f"{binary} not found — build it first: cargo build --release -p wavy-native")
for tool in ("ffmpeg", "xdotool"):
    if not shutil.which(tool):
        sys.exit(f"{tool} is required")
for port in (37391, 3737):
    with socket.socket() as s:
        if s.connect_ex(("127.0.0.1", port)) == 0:
            sys.exit(f"port {port} is busy — close the running Wavy first")

work = tempfile.mkdtemp(prefix="wavy-mem-")
home, data, media = (os.path.join(work, d) for d in ("home", "data", "media"))
wdir = os.path.join(data, "wavy")
covers = os.path.join(wdir, "covers")
for d in (home, wdir, covers, media, os.path.join(home, "run")):
    os.makedirs(d, exist_ok=True)
os.chmod(os.path.join(home, "run"), 0o700)
procs = []


def cleanup():
    for p in reversed(procs):
        if p.poll() is None:
            p.terminate()
            try:
                p.wait(5)
            except subprocess.TimeoutExpired:
                p.kill()
    if not args.keep:
        shutil.rmtree(work, ignore_errors=True)
    else:
        print(f"kept {work}")


def ff(*a):
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", *a], check=True)


# ---------- demo library ----------
print(f"generating a demo library ({args.tracks} tracks) …", flush=True)
n_covers = min(200, args.tracks)
ff("-f", "lavfi", "-i", "testsrc2=size=600x600:rate=5", "-frames:v", str(n_covers), "-q:v", "4", os.path.join(covers, "c%03d.jpg"))
song = os.path.join(media, "song.mp3")
ff("-f", "lavfi", "-i", "sine=frequency=330:duration=240", "-ac", "2", "-b:a", "128k", song)
video = os.path.join(media, "clip.mp4")
ff("-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=60", "-f", "lavfi", "-i", "sine=frequency=440:duration=60",
   "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", video)
now = int(time.time() * 1000)
tracks, ids = {}, []
for i in range(args.tracks):
    tid = f"loc_{i}"
    tracks[tid] = {"id": tid, "type": "local", "path": song, "title": f"Track {i + 1}", "artist": f"Artist {i % 60}",
                   "album": f"Album {i % 90}", "duration": 240.0, "cover": os.path.join(covers, f"c{i % n_covers + 1:03d}.jpg"),
                   "addedAt": now - i * 60_000}
    ids.append(tid)
tracks["vid"] = {"id": "vid", "type": "local", "path": video, "title": "Video clip", "artist": "Test", "duration": 60.0,
                 "video": True, "addedAt": now - 10**9}
lib = {"tracks": tracks, "playlists": [{"id": "pl0", "name": "Mix", "trackIds": ids[:100], "createdAt": now}],
       "settings": {"lang": "en", "autoUpdate": False, "volume": 0.5}}
json.dump(lib, open(os.path.join(wdir, "library.json"), "w"))
json.dump({"queue": ids[:20] + ["vid"], "qi": 0, "position": 0}, open(os.path.join(wdir, "session.json"), "w"))

env = dict(os.environ, HOME=home, XDG_DATA_HOME=data, XDG_CONFIG_HOME=os.path.join(home, ".config"),
           XDG_CACHE_HOME=os.path.join(home, ".cache"), XDG_RUNTIME_DIR=os.path.join(home, "run"))

try:
    # ---------- display ----------
    if not os.environ.get("DISPLAY"):
        if not shutil.which("Xvfb"):
            sys.exit("no $DISPLAY and no Xvfb")
        disp = next(f":{n}" for n in range(90, 200) if not os.path.exists(f"/tmp/.X11-unix/X{n}"))
        procs.append(subprocess.Popen(["Xvfb", disp, "-screen", "0", "1600x1000x24"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        env["DISPLAY"] = disp
        time.sleep(1.5)
    # ---------- audio: a real-time null sink if PulseAudio is there ----------
    audio = "ALSA null (plays faster than real time)"
    if shutil.which("pulseaudio"):
        pa = os.path.join(home, "pa.pa")
        open(pa, "w").write("load-module module-null-sink sink_name=null\nload-module module-native-protocol-unix\nset-default-sink null\n")
        procs.append(subprocess.Popen(["pulseaudio", "-n", "-F", pa, "--exit-idle-time=-1", "--daemonize=no"], env=env,
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        open(os.path.join(home, ".asoundrc"), "w").write("pcm.!default { type pulse }\nctl.!default { type pulse }\n")
        audio = "PulseAudio null sink (real time)"
        time.sleep(1.5)
    else:
        open(os.path.join(home, ".asoundrc"), "w").write("pcm.!default { type null }\n")

    # ---------- app ----------
    app = subprocess.Popen([binary], env=env, cwd=work, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    procs.append(app)
    token_file = os.path.join(wdir, "addons", "api-token")
    for _ in range(100):
        if os.path.exists(token_file):
            break
        time.sleep(0.1)
    token = open(token_file).read().strip()

    def cmd(c, **kw):
        req = urllib.request.Request("http://127.0.0.1:3737/api/v1/command", json.dumps({"cmd": c, **kw}).encode(),
                                     {"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
        urllib.request.urlopen(req, timeout=5).close()

    def state():
        req = urllib.request.Request("http://127.0.0.1:3737/api/v1/state", headers={"Authorization": f"Bearer {token}"})
        return json.load(urllib.request.urlopen(req, timeout=5))

    def xdo(*a):
        subprocess.run(["xdotool", *a], env=env, check=False)

    def mem():
        v = {}
        for line in open(f"/proc/{app.pid}/smaps_rollup"):
            k, _, rest = line.partition(":")
            if rest.strip().endswith("kB"):
                v[k] = int(rest.split()[0])
        return {"priv": v["Private_Dirty"] / 1024, "uss": (v["Private_Clean"] + v["Private_Dirty"]) / 1024, "rss": v["Rss"] / 1024}

    renderer = "unknown"
    for line in open(f"/proc/{app.pid}/maps"):
        if "llvmpipe" in line or "swrast" in line or "libgallium" in line:
            renderer = "software (Mesa llvmpipe in-process: inflates the numbers)"
            break
        if "nvidia" in line or "radeonsi" in line or "iris" in line:
            renderer = "GPU driver"

    def phase(name, during=None):
        samples, t0 = [], time.time()
        while time.time() - t0 < args.phase_sec:
            if during:
                during(time.time() - t0)
            if app.poll() is not None:
                sys.exit(f"Wavy exited during '{name}' (code {app.returncode})")
            samples.append(mem())
            time.sleep(0.5)
        peak = {k: max(s[k] for s in samples) for k in samples[0]}
        end = samples[-1]
        return {"phase": name, "peak": peak, "end": end}

    results = []
    time.sleep(4)  # startup: fonts, window, library
    results.append(phase("idle"))

    scrolled = [0]
    def scroll(t):  # wheel down through the library, then back up
        if scrolled[0] < 60:
            xdo("mousemove", "700", "450", "click", "--repeat", "3", "5")
        elif scrolled[0] < 90:
            xdo("mousemove", "700", "450", "click", "--repeat", "3", "4")
        scrolled[0] += 1
    results.append(phase("scroll library", scroll))

    cmd("play")
    time.sleep(1)
    st = state()
    if not st.get("playing"):
        print("warning: playback didn't start (no audio device?)", flush=True)
    results.append(phase("play music"))
    if os.environ.get("WAVY_MEM_DEBUG"):
        cur, rows = None, {}
        for line in open(f"/proc/{app.pid}/smaps"):
            parts = line.split()
            if parts and "-" in parts[0] and len(parts) >= 5 and not parts[0].endswith(":"):
                cur = parts[5] if len(parts) > 5 else "[anon]"
            elif parts and parts[0] == "Private_Dirty:":
                rows[cur] = rows.get(cur, 0) + int(parts[1])
        for k, v in sorted(rows.items(), key=lambda kv: -kv[1])[:12]:
            print(f"  {v / 1024:7.1f} MB  {k}")
        print("  threads:", len(os.listdir(f"/proc/{app.pid}/task")))

    for c in range(args.cycles):
        n = f" #{c + 1}" if args.cycles > 1 else ""
        cmd("jump", value=20)  # the video
        time.sleep(1.5)
        xdo("mousemove", "100", "735", "click", "1")  # open the video view (the now-playing block)
        results.append(phase("play video" + n))
        xdo("mousemove", "100", "735", "click", "1")  # close the view: video memory should be released
        cmd("jump", value=0)
        time.sleep(2)
        results.append(phase("music after video" + n))

    # ---------- report ----------
    print()
    print(f"Wavy memory (MB) — {args.tracks} tracks with covers, renderer: {renderer}, audio: {audio}")
    print(f"{'phase':<20}{'private peak':>13}{'private end':>13}{'USS peak':>10}{'RSS peak':>10}")
    for r in results:
        print(f"{r['phase']:<20}{r['peak']['priv']:>13.1f}{r['end']['priv']:>13.1f}{r['peak']['uss']:>10.1f}{r['peak']['rss']:>10.1f}")
    music = results[2]["peak"]["priv"]
    ok = music <= args.limit_mb
    print(f"\nplaying music: {music:.1f} MB private (limit {args.limit_mb:.0f} MB) — {'OK' if ok else 'OVER THE LIMIT'}")
    # GitHub Actions: the same table on the run's summary page
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as f:
            f.write(f"### Memory: {music:.1f} MB while playing music (limit {args.limit_mb:.0f} MB) {'✅' if ok else '❌'}\n\n")
            f.write(f"{args.tracks} tracks with covers · renderer: {renderer} · audio: {audio}\n\n")
            f.write("| phase | private peak, MB | private end, MB | USS peak, MB |\n|---|---:|---:|---:|\n")
            for r in results:
                f.write(f"| {r['phase']} | {r['peak']['priv']:.1f} | {r['end']['priv']:.1f} | {r['peak']['uss']:.1f} |\n")
    sys.exit(0 if ok else 1)
finally:
    cleanup()
