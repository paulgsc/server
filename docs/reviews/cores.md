# CORES: is home's code using its 4 cores, or is home at its hardware limit?

**Status:** part 1 done from the code; parts 2 and 3 wait on the owner's measurements
(the commands are in part 2). Review only: no code or policy changed. Asked 2026-11-22
by the owner, through the coordinator (run #3361). Widens paulgsc/some-ui#3348 (real-time
transcription) to everything home runs.

**Short answer before measuring:** the code and config never let home's batch and
transcription work use more than about 2 of its 4 cores at once. Most of that is
**policy** (C919's 3-core cap, LA1's pause, the 2-core render/Kokoro caps), and some of it
is **waste** (Kokoro's one thread, language detection that costs more than the hearing,
beam search nobody chose). So a 4-core box that looks "full" from the transcriber's side
is probably half idle. The measurements below confirm or refute that.

What this review could not read: paulgsc/some-ui #3348, #3309 and some-ui's CLAUDE.md
C789 ("Spend last on hardware, behind a measured judgment"). Attaching some-ui was
refused in this session. Part 3 follows the coordinator's brief for those, and its
hardware section stops at the judgment gate C789 names, without a recommendation.

---

## 1. Is the code using the cores?

Home: Intel x86_64, 4 cores (its exact model is not in either repo; measurement A1 below
gets it). Every figure here comes from a file and line in this repo at `3d3739d`.

### What can run, and on how many cores

| Work | Where | Threads it uses | Cap | Source |
|---|---|---|---|---|
| Voice-note transcriber (whisper.cpp `small` q5_1, in-process) | `file-host` | `available_parallelism() - 1` = **2** | file-host `cpus: 3.0` | `transcribe.rs` `default_threads`, `infra/compose/file_host.yml:87` |
| One note at a time, a pass of ≤ `BATCH` (20) | `file-host` | 1 note | — | `transcribe/pass.rs:58,184` |
| file_host's async runtime | `file-host` | `worker_threads = 4` | inside the same 3.0 | `main.rs:102` |
| Render (Chromium recorder, ffmpeg) | `render` | Chromium's/ffmpeg's own | `cpus: 2.0`, `cpu_shares: 256` | `call-render.yml:240-243` |
| Kokoro speaker | `kokoro` | **1** (`torch.set_num_threads(1)`, `OMP/MKL_NUM_THREADS=1`), one line at a time under a lock | `cpus: 2.0`, `cpu_shares: 256` | `infra/kokoro/say.py:74,87`, `Dockerfile.kokoro:13` |
| Batch jobs in all | `render` + `kokoro` | **one job at a time**, any kind | lane (LA1–LA4) | `docs/home-compute.md` invariant 1 |
| Rust Rounds runner | `rust-runner` | 1 program | `cpus: 1.0` | `rust_runner.yml:90` |

### Where a core sits idle

Each entry says whether it is **policy** (deliberate, written down, moved by a call) or
**waste** (nothing chose it, or what chose it no longer holds).

**P1. A note is heard on 2 threads, while 4 cores exist. (Policy, C919.)**
`file-host` is capped at 3 cores, and the transcriber takes one fewer than that, so a
core is left for requests. With nothing else running, a note uses 2 of 4 cores, and the
other 2 are close to idle (requests and polls use little; see A2's "at rest"). The
doc puts the 2-thread cost at "a little over half" of 1 thread's (`transcribe.rs` module
doc, "What it costs"), so whisper.cpp scales well to 2 threads here. Whether 3 or 4
threads still pay is measurement C.

**P2. A note pauses the render: LA1 freezes it (SIGSTOP), which idles the render's
cores. (Policy, invariant 2.)** While a note is heard, the render is frozen, not just
given less weight. Home then runs 2 transcriber threads and nothing else. `cpu_shares: 256`
already makes the render yield the CPU to anything else that wants it, so the freeze
costs ~2 cores of render progress to buy latency for a note. That latency is a backlog,
not someone waiting, as invariant 2 says itself. Whether the freeze is worth 2 cores is
measurement A4's `busy` ticks against the render backlog. Moving it touches invariant 2,
so it is the owner's call.

**P3. `TURN`/`GIVE` alternate the transcriber and the render rather than overlap them.
(Policy, TR4/C899.)** After 20 minutes of hearing while a render waits, the transcriber
hears nothing for 10 minutes (`transcribe/turn.rs:33,37`). Together with P2, home runs
either 2 transcriber threads or the render, never both. Over a long backlog that is ~2
cores of the 4 at a time, whichever side has its turn.

**P4. The render and Kokoro are each capped at 2 cores, and only one batch job runs at a
time. (Policy, invariant 1 and the compose caps.)** On an idle box a render still can't
use more than 2 cores, plus up to 2 for Kokoro. The caps are what keep the lane's memory
arithmetic and the `cpu_shares` story simple. They are the wrong unit when the box is
otherwise idle: a cap binds even when nobody else wants the CPU, which a share never does.
Measurement A3 (CFS throttling) shows whether the render actually hits its cap.

**W1. Kokoro speaks on 1 thread inside a 2-core cap. (Waste, or a reason nobody wrote
down.)** `say.py` sets `torch.set_num_threads(1)`, and the image sets
`OMP_NUM_THREADS=MKL_NUM_THREADS=1`. The lock (one line at a time) is for the seed, and
it is documented. The thread count is not. If it is for byte-identical audio (multi-threaded
reductions can reorder floats), that is a real reason: write it down, and leave it. If not,
2 threads should cut a line's time by roughly a third to a half. While the speaker
speaks, the recorder waits on it (the week's lines are said in sequence, as the recorder
in some-ui calls them; not checked here, since some-ui could not be read). So the single
thread is likely on the render's critical path. Measurement D separates the speaker's time
from the rest of a render.

**W2. Language detection costs more than the hearing. (Waste, within TR2's bound.)**
Each detection is a full encoder pass over 30 s of mel, whatever the cut's length:
"16 s for a two-second cut as for an eight-second one" on one thread
(`transcribe/engine.rs` module doc, TR2). A hearing may detect up to `DETECTS_PER_WINDOW`
(4) cuts per 30-s window of speech. On one thread that is up to ~64 s of detection per
30 s of speech, against ~39 s for the decode itself (1.3 s per second of speech). So on a
take with pauses, **detection can be more than half the transcriber's CPU.** Two cheaper
shapes exist:
- *Kinds said in one language* (Rust Rounds answers, interview takes; `held_to`,
  `voice_repo::verdict::expected_language`) are decoded in that language anyway. Their cuts
  are still detected, only as a signal for their verdict (`Measures::detected`). One
  detection, or none, would keep almost all of that signal.
- *Detection with a short encoder context:* whisper.cpp's `audio_ctx` (`set_audio_ctx` in
  whisper-rs) encodes only as many mel frames as the cut has, rather than 1500 (30 s). For
  a 2-s cut that is roughly a tenth of the encoder work. Its accuracy cost on detection is
  unmeasured here, so it is a change to check against real takes, not to ship blind.

**W3. Beam search, beam 5, on every run. (Waste until someone chooses it.)**
`engine.rs:494`: `SamplingStrategy::BeamSearch { beam_size: 5 }`. Beam 5 roughly
multiplies the decoder's work by the beam width, against greedy decoding; the encoder is
unchanged. Nothing in the code or docs says beam 5 was compared with greedy on the owner's
takes. It may be worth it (fewer garbled verdicts), but it is a cost nobody measured.
Measurement C runs both.

**W4. `worker_threads = 4` under a 3-core cap. (Small waste.)** file_host's runtime has 4
workers in a 3-core quota, plus the transcriber's 2 threads, so under load the CFS quota
throttles the container. The cost is latency spikes on requests (throttled for the rest
of a 100 ms period), not lost throughput. Measurement A3 shows file-host's throttling.

**Not a problem: blocking work on async threads.** The three heavy blocking calls are on
`spawn_blocking`: the hearing (`transcribe/pass.rs:184`), a snip's unzip (`snips.rs:617`),
and the Rust Rounds exchange (`rust_rounds/pass.rs:190`). The meaning index's embed (a
static model, `meaning.rs:139`) is called on the async thread, but it is a table lookup
and a mean, microseconds for a take's words. Nothing found here holds a runtime thread for
seconds.

**Not a problem: SQLite.** WAL, `synchronous=NORMAL`, `busy_timeout` 5 s, 5 pooled
connections, with acquire bounded (AD4; `db.rs:42-45`, `admission.rs:77`). Writes are
short appends. A busy request is answered busy rather than waiting, and
`sqlite_pool_*`/`pool_timeout` say when that happens (A5). Nothing CPU-bound waits on the
database lock.

### Summary of part 1

| | Cores idle by it | Kind | Moved by |
|---|---|---|---|
| P1 transcriber on 2 threads | ~1–2 while a note is heard alone | policy | C919 (owner) |
| P2 LA1 freezes the render for a note | ~2 render cores for every note | policy | invariant 2 (owner) |
| P3 `TURN`/`GIVE` alternate | the other side's cores during a backlog | policy | C899 constants (owner) |
| P4 render and Kokoro at 2.0 each, one job | up to 2 on an otherwise idle box | policy | compose caps / invariant 1 |
| W1 Kokoro on 1 thread | 1 of its 2 | waste or unwritten reason | a thread setting |
| W2 detection passes | none idle; work spent on a signal | waste | code (TR2's family) |
| W3 beam 5 | none idle; decoder work ×~beam | unmeasured choice | a decode setting |
| W4 4 runtime workers in 3.0 | none; throttling latency | small waste | one constant |

So the transcriber's speed on its own is probably fine for the job, and what slows home
down is that **the policies take turns rather than share** and that **W2 and W3 spend the
transcriber's cores on work the output barely needs**. That is a hypothesis until part 2.

---

## 2. What the hardware actually does: measurements for the owner

Everything below is read-only. Nothing writes to home's database, storage or containers,
except the bench (C), which writes one copy of a take to a private temporary folder and
deletes it. Run them on home, in a shell on the box (ssh). Paste the output back whole.

### A. Box and per-container CPU (cAdvisor, node_exporter, Prometheus)

**A1. The box itself** (seconds):

```sh
lscpu | grep -E '^(Model name|CPU\(s\)|Thread|Core|Socket|CPU max MHz)'
grep -o -w -E 'avx2|avx512f|avx_vnni|amx_tile|f16c|fma' /proc/cpuinfo | sort | uniq -c
free -g
lspci | grep -i -E 'vga|3d|display'
ls /dev/dri /dev/accel 2>/dev/null
docker logs file-host-server 2>&1 | grep -m1 'voice-note transcriber loaded'
```

**A2. A week of CPU, summarised per day.** Prometheus keeps 200 h, so this is the last 7
days, ending at midnight UTC each. Each line is a container, with its mean cores, its p95
cores, and the share of 5-minute slots where it was within 10 % of its cap.

```sh
P=http://127.0.0.1:9090/api/v1/query
q() { curl -sG "$P" --data-urlencode "query=$1" --data-urlencode "time=$2" \
  | python3 -c 'import json,sys; [print(" ", r["metric"].get("name", r["metric"].get("why", r["metric"].get("kind","-"))), round(float(r["value"][1]),3)) for r in json.load(sys.stdin)["data"]["result"]]'; }
U='sum by (name) (rate(container_cpu_usage_seconds_total{name=~"file-host-server|render|kokoro|rust-runner|www|orchestrator|prometheus|metabase|grafana|resume-render"}[5m]))'
for d in 0 1 2 3 4 5 6; do
  t=$(date -u -d "today -$d day" +%s)
  echo "== day ending $(date -u -d @$t +%F)"
  echo " mean cores";      q "avg_over_time(($U)[24h:5m])" $t
  echo " p95 cores";       q "quantile_over_time(0.95, ($U)[24h:5m])" $t
  echo " host busy cores (mean, p95)"
  q 'avg_over_time((sum(rate(node_cpu_seconds_total{mode!~"idle|iowait"}[5m])))[24h:5m])' $t
  q 'quantile_over_time(0.95, (sum(rate(node_cpu_seconds_total{mode!~"idle|iowait"}[5m])))[24h:5m])' $t
  echo " host CPU pressure, share of time some task waited (mean, p95)"
  q 'avg_over_time((rate(node_pressure_cpu_waiting_seconds_total[5m]))[24h:5m])' $t
  q 'quantile_over_time(0.95, (rate(node_pressure_cpu_waiting_seconds_total[5m]))[24h:5m])' $t
done
```

`python3` is on home's PATH if `which python3` says so; otherwise prefix the loop with
`nix shell nixpkgs#python3 -c bash -c '…'`, or run it from `nix develop`.

**A3. Who hits their cap (CFS throttling), over the 7 days.** The share of CPU periods
in which each container was throttled:

```sh
q 'sum by (name) (increase(container_cpu_cfs_throttled_periods_total{name=~"file-host-server|render|kokoro"}[7d])) / sum by (name) (increase(container_cpu_cfs_periods_total{name=~"file-host-server|render|kokoro"}[7d]))' $(date +%s)
```

**A4. How the lane spent the week** (ticks are 5 s, so ×5 is seconds):

```sh
echo "lane, seconds by reason";    q 'sum by (why) (increase(lane_ticks_total[7d])) * 5' $(date +%s)
echo "busy, seconds by kind";      q 'sum by (kind) (increase(lane_busy_ticks_total[7d])) * 5' $(date +%s)
echo "transcriber attempts";       q 'sum by (outcome) (increase(transcribe_attempts_total[7d]))' $(date +%s)
echo "contention excused/kept";    q 'sum by (what) (increase(contention_outcomes_total[7d]))' $(date +%s)
```

**A5. SQLite pool, to rule it out:**

```sh
q 'max_over_time((sqlite_pool_size - sqlite_pool_idle)[7d:1m])' $(date +%s)
q 'sum(increase({__name__=~".*pool_timeout.*"}[7d]))' $(date +%s)
```

**What A says:**

| Reading | Means |
|---|---|
| host busy p95 **≤ ~2.2 cores** on busy days, CPU pressure low | **the code leaves cores idle.** The box isn't the limit. Part 3's code changes apply. |
| host busy p95 **≥ ~3.6 cores** and CPU pressure p95 **> 0.2** for hours | **the cores are full.** Code changes only buy back W2/W3's share, and hardware is on the table. |
| render or kokoro throttled **> 30 %** of periods while host busy < 3 | their 2.0 caps bind on an idle box: P4 is the limit, not the hardware. |
| file-host throttled a lot while the transcriber hears | 3.0 binds: P1/W4. |
| `why="busy"` large and `kind="voice"` most of it | LA1's freeze (P2) is costing render hours. |

### B. The transcriber's real-time factor, per take length

The database is file-host's `/app/dbs/hopium.db`; find it on the host, then open it
**read only**:

```sh
DB="$(docker inspect file-host-server --format '{{range .Mounts}}{{if eq .Destination "/app/dbs"}}{{.Source}}{{end}}{{end}}')/hopium.db"
sudo python3 - "$DB" <<'PY'
import sqlite3, sys, statistics as st
db = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
rows = db.execute("""
  SELECT t.model, n.of_kind, t.outcome, t.duration_ms, t.took_ms, v.speech_ms, substr(t.heard_at,1,10)
  FROM voice_transcript t
  JOIN voice_note n USING (subject_id, note_id)
  LEFT JOIN voice_transcript_verdict v
    ON v.subject_id=t.subject_id AND v.note_id=t.note_id AND v.model=t.model AND v.attempt=t.attempt
  WHERE t.heard_at >= date('now','-30 day') AND t.duration_ms > 0
""").fetchall()
def band(ms):
    s = ms/1000
    return "<15s" if s<15 else "15-30s" if s<30 else "30-60s" if s<60 else "1-2m" if s<120 else "2m+"
groups = {}
for model, kind, outcome, dur, took, speech, day in rows:
    groups.setdefault((model, band(dur), outcome), []).append((took/dur, took/speech if speech else None, took))
print("model | length | outcome | n | RTF audio p50/p90 | RTF speech p50/p90 | took s max")
for k in sorted(groups):
    g = groups[k]; a = sorted(x[0] for x in g); s = sorted(x[1] for x in g if x[1])
    pct = lambda v,p: round(v[min(len(v)-1, int(p*len(v)))],2) if v else "-"
    print(*k, len(g), f"{pct(a,.5)}/{pct(a,.9)}", f"{pct(s,.5)}/{pct(s,.9)}", round(max(x[2] for x in g)/1000), sep=" | ")
print("\nper day: n, deadlines, mean RTF(audio)")
days = {}
for model, kind, outcome, dur, took, speech, day in rows:
    d = days.setdefault(day, [0,0,[]]); d[0]+=1; d[1]+= outcome=="deadline"; d[2].append(took/dur)
for day in sorted(days): print(day, days[day][0], days[day][1], round(st.mean(days[day][2]),2))
PY
```

It prints counts and ratios only: no note's text, id or subject. `took_ms` includes
ffmpeg's decode, the VAD, the detections and the decode, so it is the whole attempt's
real-time factor.

**The attempt log (#1087's `log_attempt`)** splits it: how much speech, and how many
detections, each took:

```sh
docker logs --since 168h file-host-server 2>&1 | grep 'voice-note transcription attempt' | tail -60
```

**What B says:** on 2 threads, the decode alone should be about 0.7× the speech (1.3 s/s
on one thread, a little over half that on two).
- **RTF(speech) ≈ 0.7–0.9, few detections:** the transcriber runs at its CPU's speed. It
  is real time for a take heard alone, and slower only when it waits for its turn.
- **RTF(speech) ≫ 1 with many `detections`:** W2 is eating the time. That is code.
- **RTF high on some days only, matching A2's busy hours or `contention_outcomes_total`:**
  it is sharing the box, not the box being slow.
- **RTF ≫ 1 on a calm box with few detections:** the CPU is slower than TR2's 2.1 GHz Xeon.
  A1's model name and flags (no `avx2` would be a large finding: the image is built for
  x86-64-v3) say why.

### C. whisper.cpp's own speed at 1, 2, 3 and 4 threads (one-off bench)

The file_host image carries no whisper.cpp program (it links it in), so this uses
nixpkgs' whisper-cpp on the host, against home's own model file, on a copy of one take.
It reads the models read-only and touches nothing of file_host's. **Run it when the lane
says idle** (with A2's `q` defined, `q lane_idle $(date +%s)` prints 1, or
`docker top render` shows no recorder), since it competes with home's own work while it runs.

```sh
# 1. One English take of 30–60 s, copied out read-only into a private folder.
T=$(mktemp -d) && chmod 700 "$T"
DB="$(docker inspect file-host-server --format '{{range .Mounts}}{{if eq .Destination "/app/dbs"}}{{.Source}}{{end}}{{end}}')/hopium.db"
sudo python3 - "$DB" "$T" <<'PY'
import sqlite3, sys
db = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
row = db.execute("""SELECT a.body, n.content_type FROM voice_transcript t
  JOIN voice_note n USING (subject_id, note_id) JOIN voice_note_audio a USING (subject_id, note_id)
  WHERE t.outcome='heard' AND t.language='en' AND t.duration_ms BETWEEN 30000 AND 60000
  ORDER BY t.heard_at DESC LIMIT 1""").fetchone()
ext = {"audio/webm": "webm", "audio/ogg": "ogg", "audio/mp4": "m4a"}[row[1]]
open(f"{sys.argv[2]}/take.{ext}", "wb").write(row[0])
PY
sudo chown "$USER" "$T"/take.*
nix shell nixpkgs#ffmpeg -c ffmpeg -loglevel error -i "$T"/take.* -ac 1 -ar 16000 "$T/take.wav"

# 2. The bench: 1–4 threads, beam 5 (home's) and greedy; whisper prints its own timings.
M=/mnt/storage/users/dev/models/whisper-models/ggml-small-q5_1.bin   # TRANSCRIBE_MODELS_DIR on home
nix shell nixpkgs#whisper-cpp -c bash -c '
  CLI=$(command -v whisper-cli || command -v whisper-cpp)
  for t in 1 2 3 4; do for bs in 5 1; do
    echo "== threads=$t beam=$bs"
    "$CLI" -m '"$M"' -f '"$T"'/take.wav -l en -t $t -bs $bs -nt -np 2>&1 \
      | grep -E "load time|mel time|encode time|decode time|batchd time|total time"
  done; done
  echo "== encoder alone, 30 s (whisper-bench)"
  B=$(command -v whisper-bench || command -v whisper-cpp-bench)
  for t in 1 2 3 4; do "$B" -m '"$M"' -t $t 2>&1 | grep -E "encode time|total time"; done'

# 3. Leave nothing behind.
rm -rf "$T"
```

If `TRANSCRIBE_MODELS_DIR` on home differs from the default, `grep TRANSCRIBE_MODELS_DIR`
in the deployment's env names it. nixpkgs' build is generic x86-64, not the image's
x86-64-v3, so absolute times may differ slightly from file_host's. The *ratios* between
thread counts and beams are what this measures.

**What C says:**
- **total(4) / total(2) ≤ ~0.65:** threads still pay, so P1's 2-thread choice leaves real
  speed on the table. With the 4th core given while nothing else runs, a take is heard ~1.5×
  faster.
- **total(4) / total(2) ≥ ~0.85:** memory bandwidth, not cores, is the wall. More threads
  (or more cores) won't help much, and the gain must come from less work (W2, W3) or a
  different kind of hardware.
- **beam 1 vs beam 5 decode time:** W3's price, to weigh against garbled verdicts.
- **encode time (30 s) at 1 thread vs TR2's 16 s:** home's CPU relative to the Xeon the
  figures were taken on.

### D. What the renders take

```sh
STORAGE=$(docker inspect render --format '{{range .Mounts}}{{if eq .Destination "/renders"}}{{.Source}}{{end}}{{end}}')
for f in "$STORAGE"/*-worker/took; do echo "$f: $(cat "$f") s"; done
docker logs --since 168h render 2>&1 | grep -E -i 'rendered|took|paused|resumed|stop' | tail -80
```

And over the same week, how much the render and speaker were each busy *while a job ran*:

```sh
q 'sum by (name) (rate(container_cpu_usage_seconds_total{name=~"render|kokoro"}[5m])) and on() (lane_batch_active == 1)' $(date +%s)
q 'avg_over_time((sum by (name) (rate(container_cpu_usage_seconds_total{name=~"render|kokoro"}[5m])) and on() (lane_batch_active == 1))[7d:5m])' $(date +%s)
```

**What D says:** if, while a job runs, render + kokoro average **well under 2 cores** and
Kokoro sits near **1.0** while render sits near 0, the job is waiting on a single-threaded
speaker (W1). If render sits at 2.0, throttled (A3), its cap binds (P4). If both are near
their caps and the host is near 4, the render is CPU-bound for real.

---

## 3. How to go about it

### If it's the code (part 2 shows idle cores)

Each change is named with its family. The expected gains are estimates from the figures
above, to be firmed up against C's numbers.

| Change | Family | Expected gain | Needs the owner? |
|---|---|---|---|
| Expected-language kinds detect once (or not at all) | TR2's detection bound (code) | Up to ~half the CPU per Rust Rounds / interview take with pauses | No: an engine change. HEARD1's verdict loses a weak signal, so say it on that issue. |
| Detect with `audio_ctx` sized to the cut | TR2 (code) | Detection ~5–10× cheaper for short cuts | No, but check it on real takes first: accuracy of detection is unmeasured. |
| Greedy decoding, or beam 2 | a decode setting | Decode ~1.5–2× faster if C bears it out | Quality call: compare garbled verdicts first. |
| Transcriber threads = 3 (or what C says pays) | a thread setting, under C919's cap | ~1.3× per take, if C's 3-thread ratio holds | Yes: C919 chose two threads with one core for requests. |
| file-host `cpus: 4.0`, as #710 had it | a compose cap (C919) | lets 3 threads hear while a core is left | Yes: C919. |
| Kokoro on 2 threads, if byte-identical audio survives it | a thread setting | each line ~1.3–2× faster, a render's speaker time with it | No, if a test proves the same bytes; else it's a determinism call. |
| `worker_threads` = the quota (3) | a constant | fewer throttling spikes on requests | No. |
| Render and Kokoro: drop the hard `cpus:` caps and keep `cpu_shares: 256` | compose caps (batch class) | a render on an idle box uses all 4 cores | Yes: it changes what the batch class means in `home-compute.md`'s table. |
| LA1 lowers the render's weight instead of freezing it, for the transcriber only | lane policy (LA1) | the render keeps ~2 cores of progress through every note | **Yes: invariant 2.** Raise on epic #705 before building. |
| Overlap rather than alternate `TURN`/`GIVE` | policy constants (C899) | backlogs and renders progress together | **Yes: invariant 2, C899.** |

None of these breaks invariant 1 (one batch job at a time) or adds a clock. The last three
change how invariant 2 is held, so they are raised, not built.

### If it's the hardware (part 2 shows full cores)

These are the classes that would remove the limit, for HWSIM's judgment gate (C789).
**No purchase is recommended here.** The figures are published, on other machines, and
part 2's numbers are what turn them into a judgment for home.

- **More CPU cores.** whisper.cpp scales with threads until memory bandwidth stops it, and
  part C's 4-thread ratio says where that is on home. faster-whisper's README reports the
  `small` model on an 8-thread i7-12700K at 1 m 42 s (int8) for its 13-minute reference
  audio, about RTF 0.13
  ([faster-whisper on PyPI](https://pypi.org/project/faster-whisper/1.0.0);
  [mirror](https://awesome.ecosyste.ms/projects/github.com%2FSYSTRAN%2Ffaster-whisper)).
  One user's independent run puts `small` int8 on CPU at 14 s for about 2 minutes of audio
  ([gist](https://gist.github.com/geekodour/8734b3bf22b8ede61fb5bfc92ce68fe3)). How many
  cores: enough that the transcriber's threads, a render and the interactive services each
  get their own. 8 would remove P1–P4's turn-taking without changing any policy.
- **An iGPU or NPU.** whisper.cpp can run its encoder on Intel GPUs through OpenVINO, and
  on GPUs through Vulkan
  ([whisper.cpp README](https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/README.md)).
  Phoronix reports whisper.cpp 1.8.3 about 12× faster on integrated graphics than CPU-only,
  measured on a Ryzen 7 6800H and a Core Ultra 7 155H
  ([Phoronix](https://phoronix.com/news/whisper-cpp-1.8.3-12x-perf)). No published
  whisper.cpp NPU figure was found. AMD documents encoder offload to its Ryzen AI NPU with
  no number ([Ryzen AI docs](https://ryzenai.docs.amd.com/en/latest/whisper_cpp.html)).
  First, A1 says whether home's own Intel CPU has an iGPU (`lspci`, `/dev/dri`). If it
  does, that is a no-purchase experiment (a Vulkan/OpenVINO build of whisper-rs), but one
  that changes the image's build.
- **A discrete GPU.** faster-whisper's README reports `large-v2` in fp16 at 54 s for the
  13-minute reference on a Tesla V100S, about RTF 0.07 (same sources). `small` needs well
  under 2 GB of VRAM, and `large-v3-turbo` a few GB. A GPU also removes the transcriber from
  the CPU budget the lane shares. It costs power and a new class of failure on a box with no
  swap.

### The no-hardware options from #3348

These come from the brief, since #3348 itself could not be read here.

| Option | What it gains | Where the audio and words live |
|---|---|---|
| A smaller model for short takes (`base`/`tiny`, or `small` greedy) | 2–4× faster on short takes, at an accuracy cost on Korean and mixed speech | Unchanged: on home, in `voice_note_audio` and `voice_transcript` |
| On-device on the phone (whisper.cpp/`tiny`–`base` on Android, or the OS recogniser) | home does no hearing for those takes; the words come back in real time | The audio is heard on the phone. The words go to home as text. The audio still goes home if kept (Studio), or never leaves the phone if not. An OS recogniser may send audio to the vendor's cloud, a privacy change for `docs/identity.md` |
| Streaming while recording (chunks sent and heard as they arrive) | the words are ready when the take ends; same total CPU, spread out | Audio reaches home in chunks during the take, not after. A partial take exists on home before it is complete. The "audio touches disk once" promise needs rewording |

---

## What finishes this review

From the owner's output:
1. **A1:** CPU model, flags (`avx2`?), RAM, any iGPU or NPU.
2. **A2/A3:** host busy cores and CPU pressure on the busiest day, and which containers throttle.
3. **A4:** lane seconds by reason, and `busy` by kind.
4. **B:** RTF by length (audio and speech), deadlines per day, and ~60 attempt log lines
   (for `detections`).
5. **C:** the thread and beam table.
6. **D:** render `took` and the render/kokoro split while a job runs.

With those, part 2 gets its verdict (code, policy or hardware), part 3's table gets its
numbers, and the policy rows go to the owner as calls.
