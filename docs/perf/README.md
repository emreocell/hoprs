# Performance (spec §9)

The v1 targets of the design spec (§9), how each one is measured, and the last numbers.
`scripts/perf.sh` runs every check; `.github/workflows/perf.yml` runs the same script on
`workflow_dispatch`, on every push to `main` and weekly, and puts the tables in the job
summary. It is a separate workflow, never a required pull-request check: shared runners
are noisy, so a miss fails the workflow without blocking a merge.

## The checks

| Target (spec §9) | Check | Criterion |
|---|---|---|
| Open in < 500 ms | `crates/hoprs-app/benches/startup.rs`: the release binary with `HOPRS_PROBE_STARTUP=1`, 5 runs under `xvfb-run`, tiny-skia renderer, fresh home; median | `startup_ms < 500` |
| Idle memory < 80 MB | The same probe, 2 s after the first frame, no session (`VmRSS`) | `rss_mb < 80` |
| 100k entries scroll at ≥ 60 fps | `crates/hoprs-app/benches/ui.rs` (criterion): `view()` of a tab whose pane lists 100 000 entries, plus the layout of the virtual list and the draw, at random offsets; p95 of 1 000 frames | p95 < 16 ms |
| SFTP 1 GB at 50 ms RTT ≥ 90% of `scp` | `crates/hoprs-transfer/benches/network.rs`: upload of a 1 GiB file by `TransferEngine` + `SftpFs` (4 workers) against `scp`, same latency, same file; median of 3 | hoprs ≥ 0.9 × `scp` (throughput) |
| 10k small files (SFTP and FTP) no worse than `lftp` | The same harness: 10 000 files of 4 KiB, 4 workers, against `lftp mirror -R --parallel=4`, over SFTP and over plain FTP, same latency; median of 3 | hoprs ≤ 1.0 × `lftp` (time) |
| Progress ≤ 10 Hz aggregated | `progress_rate_at_most_10hz` (`crates/hoprs-transfer/tests/engine_files.rs`): `Progress` events during 3 s of a 4-worker folder upload whose files keep finishing | ≤ 31 events in 3 s |

### Start-up probe

With `HOPRS_PROBE_STARTUP=1` the app starts as usual, then prints on stdout, and exits:

```
startup_ms=<n> rss_mb=<m>     # when the first frame has been drawn
idle_rss_mb=<m>               # 2 s later, idle, no session
```

`startup_ms` runs from the start of `hoprs_app::run()` (the first line of `main`) to the
first message iced handles after the first `RedrawRequested`, that is once that frame is
drawn and presented. The bench also reports the time from spawning the process to reading
that line (exec and dynamic loading included). Memory is `VmRSS` from `/proc/self/status`,
in MB (10^6 bytes). `hoprs_app::bench::parse_probe_line` reads the lines back.

### Network bench

`docker/compose.bench.yml` (project `hoprs-bench`, ports 2422 and 2421, passive ports
33000-33019) runs the images of the test environment with `NET_ADMIN`. Latency comes from
`tc netem` inside each container, on everything the container sends: a delay equal to the
whole round trip, one way (netem on `eth0` only shapes egress). The FTP server is on a
bridge network (netem cannot go on the host's interfaces, which the test FTP server
shares) and announces `pasv_address=127.0.0.1`, where its passive ports are published.
Uploads land on a tmpfs, so the server's disk is not what is measured.

The harness sets the latency per scenario (`tc qdisc replace`), empties the remote folder
before every run, checks afterwards that every byte or every file arrived, and alternates
which side goes first. Connecting counts in hoprs' time, as it does in the tools'. The
baselines authenticate with the fixture key `docker/fixtures/id_ed25519` (copied to a 0600
file); `lftp` reaches SFTP through `sftp:connect-program "ssh -a -x -i <key> ..."`. No
password prompt anywhere. The 1 GiB file is sparse.

The small files run at **10 ms** RTT by default, not 50 ms: at 50 ms the three small-file
comparisons (hoprs and `lftp`, three runs each) take about three hours.
`HOPRS_BENCH_SMALL_RTT_MS=50` reproduces the big file's latency.

Settings (`HOPRS_BENCH_*`, defaults match the compose file):

| Variable | Default | |
|---|---|---|
| `HOPRS_BENCH_NETWORK` | unset | `1` runs the bench; otherwise it skips (so `cargo bench -p hoprs-transfer` stays local) |
| `HOPRS_BENCH_COMPOSE` | `docker/compose.bench.yml` | |
| `HOPRS_BENCH_HOST` | `127.0.0.1` | |
| `HOPRS_BENCH_SSH_PORT`, `_SSH_USER`, `_SSH_KEY` | `2422`, `alice`, `docker/fixtures/id_ed25519` | |
| `HOPRS_BENCH_FTP_PORT`, `_FTP_USER`, `_FTP_PASSWORD` | `2421`, `frank`, `frank-pass` | |
| `HOPRS_BENCH_RTT_MS` | `50` | big file (and the containers' starting delay) |
| `HOPRS_BENCH_SMALL_RTT_MS` | `10` | small files |
| `HOPRS_BENCH_BIG_MIB` | `1024` | |
| `HOPRS_BENCH_SMALL_FILES` | `10000` | |
| `HOPRS_BENCH_RUNS` | `3` | |
| `HOPRS_BENCH_ONLY` | `sftp-big,sftp-small,ftp-small` | |

## Running locally

```sh
sudo apt install xvfb lftp openssh-client   # once
scripts/perf.sh                             # about 45 min; summary in target/perf/summary.md
```

The script starts `docker/compose.bench.yml` (beside the test environment, which it does
not touch) and stops it at the end (`HOPRS_PERF_KEEP=1` keeps it). Single checks:

```sh
cargo bench -p hoprs-app --bench ui
xvfb-run -a cargo bench -p hoprs-app --bench startup
docker compose -f docker/compose.bench.yml up -d --build --wait
HOPRS_BENCH_NETWORK=1 HOPRS_BENCH_SMALL_FILES=1000 HOPRS_BENCH_RUNS=1 \
  cargo bench -p hoprs-transfer --bench network
cargo test -p hoprs-transfer --test engine_files progress_rate_at_most_10hz
```

The bench exits with status 1 when a target is missed, 2 when it cannot run.

## Last numbers

Local run, 2026-11-13, branch `plan-6-terminal-polish` with the fixes below (Ubuntu
26.04, 12 CPUs; Docker 29; `lftp` 4.9.2 and Xvfb from an Ubuntu 24.04 container, `scp`
OpenSSH 10 on the host, sshd OpenSSH 9.2 in the container). The first CI run of
`perf.yml` adds its numbers to its job summary.

| Target (spec §9) | Measured | Criterion | Result |
|---|---|---|---|
| Open < 500 ms | 94 ms (spawn to first frame: 97 ms) | `startup_ms < 500` | pass |
| Idle memory < 80 MB | 34.1 MB, 2 s after the first frame | `rss_mb < 80` | pass |
| 100k entries ≥ 60 fps | p95 0.65 ms per frame (max 1.22 ms) | p95 < 16 ms | pass |
| SFTP 1 GiB, 50 ms RTT | hoprs 36.3 MiB/s, `scp` 36.3 MiB/s (28.2 s each) | ≥ 0.9 × `scp` | pass (1.00) |
| 10k × 4 KiB, SFTP, 10 ms RTT | hoprs 183.8 s, `lftp` 82.7 s | ≤ 1.0 × `lftp` | **fail (2.22)** |
| 10k × 4 KiB, FTP, 10 ms RTT | hoprs 347.7 s, `lftp` 155.0 s | ≤ 1.0 × `lftp` | **fail (2.24)** |
| Progress ≤ 10 Hz | `progress_rate_at_most_10hz` passes | ≤ 31 events in 3 s | pass |

Both 1 GiB uploads are bound by the same thing (the SSH channel window over a 50 ms round
trip), hence the identical times.

### Fixed while measuring

- **Progress rate.** Every finished copy sent its final count at once: a folder of small
  files gave about 80 `Progress` events per second (238 in 3 s in the test). Final counts
  now go out at once only when the last event is an interval old; otherwise with the next
  tick (`hoprs-transfer`, `Actor::finish_progress`).
- **FTP per-file metadata.** The engine's `stat` of each file (`SIZE`, `STAT`, `MDTM`) and
  the `MFMT` after it queued, one at a time, on the control connection behind all four
  workers. They now run on an idle transfer connection back in the pool for less than
  2 s when one is free, and such a connection is reused without a `NOOP`. 1 000 files at
  10 ms: 56 s before, 27 s after (`lftp`: 16 s).

### Still missed: small files

hoprs spends more round trips per file than `lftp` (counted from the code, consistent
with the times: about 7 for SFTP against `lftp`'s 4, about 10 for FTP against about 6):

- **SFTP:** the engine's conflict check (`lstat` of the destination), `open_write`'s own
  `lstat` (it refuses to open a FIFO), open, write, close, the size check after the copy
  (`lstat`), and the modification time (`setstat`). `lftp` opens, writes and closes.
- **FTP:** the conflict check (`SIZE` + `STAT` for a missing file), `PASV`, `STOR`, the
  final reply, the size check (`SIZE` + `STAT` + `MDTM`), and `MFMT`.

The extra round trips are the engine's safety checks (no silent overwrite, size verified,
times kept), not slack in the protocols. Closing the gap means changing those contracts,
for example skipping the conflict check under a folder the run itself created, letting
`open_write` trust a fresh `stat`, verifying the size from the writer's own acknowledged
count instead of a `stat`, or setting the time on the open handle. That is
follow-up work for the transfer engine, not a cheap fix.
