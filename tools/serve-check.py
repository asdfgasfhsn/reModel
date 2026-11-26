#!/usr/bin/env python3
"""The acceptance test of `remodel serve` (make serve-check; docs/uplift/scenes-and-server.md,
"The live session"): the real binary, spoken to as an MCP client over its
standard input and output and over its socket, on a real game.

  serve-check.py REMODEL GAME ROMDIR WORKDIR [--measure]

It checks, in order: initialize and tools/list; a session opened at power-on
and stepped along a scene's recipe ends on the scene's recorded state hash,
and so does a session opened from the scene; mark, step, restore gives the
mark's hash; the exported tour, run by `remodel tour run`, gives the
session's hash; read; render writes a PNG; `remodel call` over the socket
gives the same envelope; closing standard input stops the server and
removes the socket. With --measure it also prints the numbers the design
doc records (step speed with and without drawing, render latency warm,
mark cost, memory a session holds).
The binary is run through tools/capped by the caller; this script only
spawns what it is given.
"""
import json
import os
import re
import subprocess
import sys
import time

remodel, game, rom, work = sys.argv[1:5]
measure = "--measure" in sys.argv
os.makedirs(work, exist_ok=True)
sock = os.path.join(work, "serve.sock")
data = os.path.join(work, "data")
env = dict(os.environ, REMODEL_DATA=data, REMODEL_SCRATCH=work)
base = remodel.split() + ["--game", game, "--rom", rom]


def run(*args, ok=True):
    r = subprocess.run(base + list(args), capture_output=True, text=True, env=env)
    if ok and r.returncode != 0:
        sys.exit(f"serve-check: remodel {' '.join(args)} failed ({r.returncode}):\n{r.stdout}{r.stderr}")
    return r


def fail(msg):
    sys.exit(f"serve-check: FAILED: {msg}")


# The scene whose recipe the session replays: the earliest the game has.
scenes = json.loads(run("scene", "list", "--json").stdout)["data"]["scenes"]
scene = min(scenes, key=lambda s: s["frame"])
name, frame = scene["name"], scene["frame"]
run("scene", "make", name)
meta = dict(
    (k.strip(), v.strip())
    for k, v in (l.split("=", 1) for l in open(os.path.join(data, game, "scenes", name, "scene.txt")) if "=" in l)
)
scene_hash = meta["state_hash"]
coin = re.search(r"coin-start = (.*)", run("inputs").stdout).group(1).strip()
script = []
for item in meta["input"].split(","):
    script += coin.split(",") if item.strip() == "coin-start" else [item.strip()]
print(f"scene {name}, frame {frame}, recorded hash {scene_hash}")


def held_at(f):
    out = []
    for it in script:
        what, r = it.rsplit("@", 1)
        a, _, b = r.partition("-")
        if int(a) <= f <= int(b or a):
            out.append(what)
    return out


srv = subprocess.Popen(
    base + ["--out", os.path.join(work, "serve"), "serve", "--socket", sock, "--max-step", "3600"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(os.path.join(work, "serve.err"), "w"),
    text=True, bufsize=1, env=env,
)
ids = iter(range(1, 10**6))


def rpc(method, params=None, notify=False):
    msg = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        msg["params"] = params
    if not notify:
        msg["id"] = next(ids)
    srv.stdin.write(json.dumps(msg) + "\n")
    srv.stdin.flush()
    if notify:
        return None
    line = srv.stdout.readline()
    if not line:
        fail("the server closed its output")
    return json.loads(line)


def call(verb, **args):
    r = rpc("tools/call", {"name": verb, "arguments": args})
    if "error" in r:
        fail(f"{verb}: JSON-RPC error {r['error']}")
    env_ = r["result"]["structuredContent"]
    if json.loads(r["result"]["content"][0]["text"]) != env_:
        fail(f"{verb}: the text content is not the envelope")
    return env_


def need(verb, **args):
    e = call(verb, **args)
    if not e["ok"]:
        fail(f"{verb} {args}: {e['error']}")
    return e["data"]


def drive(sid, to, start=0):
    """Step a session along the recipe: before each stretch of frames with the
    same inputs, say what is held, then step it."""
    c = start
    while c < to:
        h = held_at(c)
        n = 1
        while c + n < to and held_at(c + n) == h:
            n += 1
        need("session.input", session=sid, inputs=h)
        need("session.step", session=sid, frames=n)
        c += n
    return need("session.input", session=sid, inputs=held_at(to))


r = rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "serve-check", "version": "1"}})
if r["result"]["serverInfo"]["name"] != "remodel":
    fail("initialize")
rpc("notifications/initialized", notify=True)
tools = [t["name"] for t in rpc("tools/list")["result"]["tools"]]
want = ["session.open", "session.input", "session.step", "session.mark", "session.restore", "session.forget",
        "session.read", "session.watch", "session.disasm", "session.moves", "session.list", "session.poke", "session.camera", "session.render", "session.export", "session.close", "spine.index", "spine.where", "session.probe", "scene.probe", "scene.render"]
if tools != want:
    fail(f"tools/list {tools}")
if "error" not in rpc("tools/call", {"name": "session.nonesuch", "arguments": {}}):
    fail("an unknown tool is a JSON-RPC error")
print(f"mcp: {len(tools)} tools")

# From power-on, along the recipe, to the scene's frame.
t = time.time()
sid = need("session.open", **{"from": "power-on"})["session"]
last = drive(sid, frame)
step_secs = time.time() - t
if last["frame"] != frame:
    fail(f"at frame {last['frame']}, not {frame}")
state = need("session.step", session=sid, frames=1)  # one on, to see the hash change
fresh = need("session.open", **{"from": "power-on"})["session"]
drive(fresh, frame)
h = need("session.mark", session=fresh, name="at-scene")["state_hash"]
if h != scene_hash:
    fail(f"hash at the scene's frame {h}, the scene's {scene_hash}")
print(f"power-on to frame {frame} along the recipe: hash {h} = the scene's ({frame / step_secs:.0f} frames/s with the other session's work)")
need("session.close", session=fresh)

# From the scene's state.
s2 = need("session.open", **{"from": f"scene:{name}"})
if s2["state_hash"] != scene_hash or s2["frame"] != frame:
    fail(f"opened from the scene at {s2['frame']} with hash {s2['state_hash']}")
print(f"open from scene:{name}: hash {s2['state_hash']} in {s2['ms']:.0f} ms")
need("session.close", session=s2["session"])

# Mark, step, restore.
m = need("session.mark", session=sid, name="a")
st = need("session.step", session=sid, frames=45, inputs=["start"])
if st["state_hash"] == m["state_hash"]:
    fail("stepping did not change the state")
back = need("session.restore", session=sid, name="a")
if back["state_hash"] != m["state_hash"] or back["frame"] != m["frame"]:
    fail("restore did not give the mark's state")
again = need("session.step", session=sid, frames=45, inputs=["start"])
if again["state_hash"] != st["state_hash"]:
    fail("the same inputs from the same mark gave another state")
print(f"mark, step 45, restore: hash {back['state_hash']} = the mark's; the same step again repeats")

# Limits.
e = call("session.step", session=sid, frames=3601)
if e["ok"] or e["code"] != 2 or "--max-step" not in e["error"]:
    fail("the step limit")
e = call("session.step", session=sid)
if e["ok"] or "frames" not in e["error"]:
    fail("argument validation")

# Export, and replay as a tour.
exp = need("session.export", session=sid, label="serve-check", file="serve-check")
tdir = os.path.join(work, "tour")
r = run("tour", "run", "serve-check", "--tours", exp["file"], "--dir", tdir, "--json")
row = json.loads(r.stdout)["data"]["tours"][0]
if row["state_hash"] != exp["state_hash"]:
    fail(f"the exported tour ends on {row['state_hash']}, the session on {exp['state_hash']}")
print(f"export: {exp['line'][:70]}...  tour run -> {row['state_hash']} = the session's")

# Read.
vals = need("session.read", session=sid)
raw = need("session.read", session=sid, names=["0x500000:b"])
print(f"read: {len(vals['values'])} named values, raw ok ({raw['values'][0]['raw']})")

# Render, if there is a GPU.
shot = call("session.render", session=sid, view="540,16:9", name="serve-check")
if shot["ok"]:
    if open(shot["data"]["picture"], "rb").read(4)[1:4] != b"PNG":
        fail("render wrote no PNG")
    print(f"render: {shot['data']['picture']} in {shot['data']['ms']:.0f} ms (draw {shot['data']['ms_draw']:.0f})")
elif shot["code"] == 4 and "GPU" in shot["error"]:
    print("render: no GPU, skipped")
else:
    fail(f"render: {shot['error']}")

# A still of the made scene, the GPU kept warm.
still = call("scene.render", scene=name, view="540,16:9")
if still["ok"]:
    if open(still["data"]["picture"], "rb").read(4)[1:4] != b"PNG":
        fail("scene.render wrote no PNG")
    print(f"scene.render: {still['data']['picture']} in {still['data']['ms']:.0f} ms")
elif not (still["code"] == 4 and "GPU" in still["error"]):
    fail(f"scene.render: {still['error']}")
# What draws a pixel: from the made scene, and from the live session's frame.
pr = call("scene.probe", scene=name, at="0,0,496,384")
if not pr["ok"] or not pr["data"]["sources"]:
    fail(f"scene.probe: {pr}")
live = call("session.probe", session=sid, at="0,0,16,16")
if not live["ok"] or live["data"]["pixels_total"] != 256:
    fail(f"session.probe: {live}")
if call("scene.probe", scene=name, at="9999,0")["code"] != 2:
    fail("a probe outside the frame is a bad request")
e = call("scene.render", scene="nonesuch")
if e["ok"] or e["code"] != 2:
    fail("an unknown scene is a bad request")

# The same verbs from the command line, over the socket.
cli = subprocess.run(base + ["call", "session.read", f"session={sid}", "names=0x500000:b", "--socket", sock, "--json"],
                     capture_output=True, text=True, env=env)
ans = json.loads(cli.stdout)
if cli.returncode != 0 or ans["data"]["values"][0]["raw"] != raw["values"][0]["raw"] or ans["data"]["frame"] != raw["frame"]:
    fail(f"remodel call: {cli.stdout}{cli.stderr}")
bad = subprocess.run(base + ["call", "session.step", f"session={sid}", "frames=9999", "--socket", sock, "--json"],
                     capture_output=True, text=True, env=env)
if bad.returncode != 2 or json.loads(bad.stdout)["code"] != 2:
    fail("the CLI's exit code for a refused step is 2")
print("remodel call over the socket: the same envelope, exit codes 0 and 2")
if oct(os.stat(sock).st_mode & 0o777) != "0o600":
    fail("the socket is not mode 0600")

# Verb help from the table, the session list, a watch that changes nothing, a disassembly.
hp = subprocess.run(base + ["call", "session.step", "--help"], capture_output=True, text=True, env=env)
if hp.returncode != 0 or "follow" not in hp.stdout or "until" not in hp.stdout:
    fail(f"remodel call VERB --help: {hp.stdout}{hp.stderr}")
lst = need("session.list")
if sid not in [s["session"] for s in lst["sessions"]]:
    fail(f"session.list does not show {sid}: {lst}")
need("session.mark", session=sid, name="chk")
need("session.input", session=sid, inputs=[])
plain = need("session.step", session=sid, frames=30)
need("session.restore", session=sid, name="chk")
need("session.input", session=sid, inputs=[])
# Moves: the game's table lists, and a move steps like its raw inputs.
mv = need("session.moves", session=sid, where="all")
if mv["count"] < 5:
    fail(f"session.moves: {mv}")
need("session.restore", session=sid, name="chk")
need("session.input", session=sid, inputs=[])
by_move = need("session.step", session=sid, move=["wait-10", "steer-left-hi-5f"])
if by_move["frames_run"] < 15:
    fail(f"step by move: {by_move}")
need("session.restore", session=sid, name="chk")
need("session.input", session=sid, inputs=[])
need("session.watch", session=sid, what=["0x500000:0x10000"], limit=50)
watched = need("session.step", session=sid, frames=30)
need("session.watch", session=sid, what=[])
if watched["state_hash"] != plain["state_hash"]:
    fail("a watch changed the state hash")
hits = watched["watch"]["hits"]
if not hits:
    fail("a watch of the work RAM saw no writes in 30 frames")
dis = need("session.disasm", session=sid, addr=hits[0]["ip"], count=4)
if len(dis["listing"]) != 4:
    fail(f"disasm: {dis}")
rng = need("session.read", session=sid, names=["0x500000:16", "0x500000:4:l:float"], format="compact")
if len(rng["values"]["0x500000:16"]) != 32:
    fail(f"a range read: {rng}")
print(f"watch: {watched['watch']['writes']} writes in 30 frames, state hash unchanged; disasm at {hits[0]['ip']}: {dis['listing'][0].strip()}")

if measure:
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    need("session.input", session=sid, inputs=[])
    # Step speed with no drawing.
    t = time.time()
    need("session.step", session=sid, frames=600)
    plain = 600 / (time.time() - t)
    # With a picture after every 10 frames.
    if shot["ok"]:
        t = time.time()
        for i in range(10):
            need("session.step", session=sid, frames=10)
            need("session.render", session=sid, view="1080", name="m")
        drawn = 100 / (time.time() - t)
    else:
        drawn = None
    # Render latency warm (the GPU started above).
    lat = []
    if shot["ok"]:
        for i in range(5):
            t = time.time()
            need("session.render", session=sid, view="1080", name="m")
            lat.append((time.time() - t) * 1e3)
        last = need("session.render", session=sid, view="1080", name="m")
        parts = {k: last[k] for k in ("ms", "ms_draw", "ms_write", "ms_lock", "ms_gpu_init")}
        sl = []
        for i in range(5):
            t = time.time()
            d = need("scene.render", scene=name, view="1080")
            sl.append((round((time.time() - t) * 1e3), d["ms_draw"], d["ms_write"]))
    # Mark cost and memory.
    mk = []
    for i in range(5):
        t = time.time()
        d = need("session.mark", session=sid, name=f"m{i}")
        mk.append(d["ms"])
    t = time.time()
    need("session.restore", session=sid, name="m0")
    rs = (time.time() - t) * 1e3
    rss = lambda: int(open(f"/proc/{srv.pid}/statm").read().split()[1]) * 4096 / 2**20
    r0 = rss()
    s3 = need("session.open", **{"from": "power-on"})
    r1 = rss()
    for i in range(3):
        need("session.mark", session=s3["session"], name=f"k{i}")
    r2 = rss()
    print(f"MEASURE step with no drawing: {plain:.0f} frames/s; with a 1080p picture every 10 frames: "
          f"{drawn if drawn is None else round(drawn)} frames/s; render 1080p warm (round trip): "
          f"{[round(x) for x in lat]} ms, parts {parts}; scene.render 1080p warm (total, draw, write) {sl} ms; mark {mk} ms; restore {rs:.0f} ms; "
          f"memory: session +{r1 - r0:.0f} MB, marks +{(r2 - r1) / 3:.0f} MB each, server {r2:.0f} MB")

need("session.close", session=sid)
srv.stdin.close()
try:
    srv.wait(timeout=20)
except subprocess.TimeoutExpired:
    srv.kill()
    fail("the server did not stop when its input closed")
if os.path.exists(sock):
    fail("the socket was left behind")
print("closing the input stopped the server and removed the socket")
print("serve-check: ok")
