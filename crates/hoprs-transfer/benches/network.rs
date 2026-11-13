//! Spec §9 network targets, against `docker/compose.bench.yml` (netem latency inside the
//! containers), hoprs (`TransferEngine` + `SftpFs` / `FtpFs`, 4 workers) against the usual
//! command-line tools under the same latency, on the same files:
//!
//! - one 1 GiB file over SFTP at 50 ms RTT: hoprs ≥ 0.9 × the throughput of `scp`;
//! - 10 000 files of 4 KiB over SFTP and over FTP, at 10 ms RTT: hoprs takes no longer
//!   than `lftp mirror -R --parallel=4`.
//!
//! `HOPRS_BENCH_KNOWN_GAPS` (comma-separated: `big_sftp`, `small_sftp`, `small_ftp`) lists
//! targets known to be missed: still measured and printed, marked "known gap", not fatal.
//!
//! Each side runs [`Config::runs`] times (median), alternating which goes first. Prints a
//! Markdown table on stdout (progress on stderr) and exits with status 1 when a target is
//! missed, 2 when the bench cannot run.
//!
//! Runs only with `HOPRS_BENCH_NETWORK=1` (`scripts/perf.sh` sets it): `cargo bench -p
//! hoprs-transfer` alone runs the engine benches and skips this one. Settings come from
//! `HOPRS_BENCH_*` variables (see [`Config::from_env`] and `docs/perf/README.md`), with
//! defaults that match the compose file. Needs `docker`, `scp` and `lftp` on `PATH`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoprs_core::{RemotePath, SiteId};
use hoprs_remote::{FtpConfig, FtpFs, FtpSecurity, LocalFs, RemoteFs, ScriptedFtpPrompter};
use hoprs_remote::{SftpConfig, SftpFs};
use hoprs_ssh::{KeySource, Prompter, ScriptedPrompter, SshConfig, SshSession};
use hoprs_transfer::{
    Direction, EngineConfig, EventKind, QueueStore, SiteEndpoints, TransferEngine, TransferId,
    TransferSpec, TransferState,
};
use tokio::runtime::Runtime;
use tokio::sync::broadcast::error::RecvError;

const WORKERS: usize = 4;
const SMALL_FILE_LEN: usize = 4 * 1024;
/// One side of one run never takes longer.
const RUN_TIMEOUT: Duration = Duration::from_secs(1800);
const SSH_DIR: &str = "/home/alice/bench";
const FTP_DIR: &str = "/home/frank/bench";

/// Settings, from `HOPRS_BENCH_*`.
struct Config {
    /// `docker compose` file of the bench services.
    compose: PathBuf,
    host: String,
    ssh_port: u16,
    ssh_user: String,
    /// Private key authorized for `ssh_user` (copied to a 0600 file before use).
    ssh_key: PathBuf,
    ftp_port: u16,
    ftp_user: String,
    ftp_password: String,
    /// Size of the big file, MiB.
    big_mib: u64,
    small_files: usize,
    /// Round-trip time added by netem for the big file, ms.
    rtt_ms: u64,
    /// Round-trip time added by netem for the small files, ms. 10 by default: at 50 the
    /// small-file comparisons alone take about three hours.
    small_rtt_ms: u64,
    runs: usize,
    /// Scenarios to run (all by default).
    only: Vec<String>,
    /// Targets known to be missed (`small_sftp`, `small_ftp`, `big_sftp`): still measured
    /// and printed, marked as a known gap, and not failing the bench.
    known_gaps: Vec<String>,
}

impl Config {
    fn from_env() -> Self {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let var = |name: &str, default: &str| {
            std::env::var(format!("HOPRS_BENCH_{name}")).unwrap_or_else(|_| default.to_owned())
        };
        let num = |name: &str, default: u64| {
            var(name, &default.to_string())
                .parse::<u64>()
                .unwrap_or_else(|_| fail_setup(&format!("HOPRS_BENCH_{name} is not a number")))
        };
        let rtt_ms = num("RTT_MS", 50);
        Self {
            compose: PathBuf::from(var(
                "COMPOSE",
                &repo.join("docker/compose.bench.yml").to_string_lossy(),
            )),
            host: var("HOST", "127.0.0.1"),
            ssh_port: num("SSH_PORT", 2422) as u16,
            ssh_user: var("SSH_USER", "alice"),
            ssh_key: PathBuf::from(var(
                "SSH_KEY",
                &repo.join("docker/fixtures/id_ed25519").to_string_lossy(),
            )),
            ftp_port: num("FTP_PORT", 2421) as u16,
            ftp_user: var("FTP_USER", "frank"),
            ftp_password: var("FTP_PASSWORD", "frank-pass"),
            big_mib: num("BIG_MIB", 1024),
            small_files: num("SMALL_FILES", 10_000) as usize,
            rtt_ms,
            small_rtt_ms: num("SMALL_RTT_MS", 10),
            runs: num("RUNS", 3).max(1) as usize,
            only: var("ONLY", "sftp-big,sftp-small,ftp-small")
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
            known_gaps: var("KNOWN_GAPS", "")
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }
}

fn fail_setup(message: &str) -> ! {
    eprintln!("network bench: {message}");
    std::process::exit(2);
}

fn main() {
    if std::env::var_os("HOPRS_BENCH_NETWORK").is_none_or(|v| v != "1") {
        eprintln!(
            "network bench: skipped (needs HOPRS_BENCH_NETWORK=1 and docker/compose.bench.yml; \
             see docs/perf/README.md)"
        );
        return;
    }
    let cfg = Config::from_env();
    for gap in &cfg.known_gaps {
        if !["big_sftp", "small_sftp", "small_ftp"].contains(&gap.as_str()) {
            fail_setup(&format!("unknown target {gap:?} in HOPRS_BENCH_KNOWN_GAPS"));
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKERS)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let work = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("bench work dir");
    let key = install_key(&cfg.ssh_key, work.path());
    let files = Files::create(work.path(), &cfg);

    let mut results = Vec::new();
    for scenario in &cfg.only {
        let (gap, result) = match scenario.as_str() {
            "sftp-big" => ("big_sftp", sftp_big(&cfg, &rt, &files, &key)),
            "sftp-small" => ("small_sftp", sftp_small(&cfg, &rt, &files, &key)),
            "ftp-small" => ("small_ftp", ftp_small(&cfg, &rt, &files)),
            other => fail_setup(&format!("unknown scenario {other:?} in HOPRS_BENCH_ONLY")),
        };
        results.push((cfg.known_gaps.iter().any(|g| g == gap), gap, result));
    }

    println!(
        "### Network (median of {}, netem in the containers)\n",
        cfg.runs
    );
    println!("| Target (spec §9) | RTT | hoprs | Baseline | Ratio | Criterion | Result |");
    println!("|---|---|---|---|---|---|---|");
    for (known, gap, r) in &results {
        let verdict = match (r.pass, known) {
            (true, false) => "pass".to_owned(),
            // Met again: it should leave the allow-list.
            (true, true) => format!("pass (`{gap}` is still listed as a known gap)"),
            (false, true) => "known gap (see follow-up)".to_owned(),
            (false, false) => "**FAIL**".to_owned(),
        };
        println!(
            "| {} | {} ms | {} | {} | {:.3} | {} | {verdict} |",
            r.target, r.rtt_ms, r.hoprs, r.baseline, r.ratio, r.criterion,
        );
    }
    if results.iter().any(|(known, _, r)| !r.pass && !known) {
        std::process::exit(1);
    }
}

/// One row of the table.
struct Outcome {
    target: &'static str,
    rtt_ms: u64,
    hoprs: String,
    baseline: String,
    ratio: f64,
    criterion: &'static str,
    pass: bool,
}

/// The local files every run uploads.
struct Files {
    /// Root of the `LocalFs` (and the tools' working directory).
    root: PathBuf,
    big: &'static str,
    big_len: u64,
    tree: &'static str,
}

impl Files {
    fn create(work: &Path, cfg: &Config) -> Self {
        let root = work.join("local");
        std::fs::create_dir_all(root.join("tree")).expect("local tree");
        // Sparse: no disk space, reads back as zeros (SSH compression is off on both sides).
        let big_len = cfg.big_mib << 20;
        std::fs::File::create(root.join("big.bin"))
            .and_then(|f| f.set_len(big_len))
            .expect("big file");
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut payload = vec![0u8; SMALL_FILE_LEN];
        for i in 0..cfg.small_files {
            for byte in &mut payload {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *byte = x as u8;
            }
            std::fs::write(root.join(format!("tree/f{i:05}.bin")), &payload).expect("small file");
        }
        Self {
            root,
            big: "big.bin",
            big_len,
            tree: "tree",
        }
    }

    fn local(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

/// The private key in a 0600 file, as `ssh` demands.
fn install_key(source: &Path, work: &Path) -> PathBuf {
    let key = work.join("id_bench");
    std::fs::copy(source, &key)
        .unwrap_or_else(|e| fail_setup(&format!("cannot read {}: {e}", source.display())));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    key
}

// ---- Scenarios ------------------------------------------------------------------------

fn sftp_big(cfg: &Config, rt: &Runtime, files: &Files, key: &Path) -> Outcome {
    set_rtt(cfg, "sshd", cfg.rtt_ms);
    let remote = format!("{SSH_DIR}/{}", files.big);
    let verify = |label: &str| {
        let size = compose_exec(cfg, "sshd", &format!("stat -c %s {remote}"));
        assert_eq!(
            size.trim(),
            files.big_len.to_string(),
            "{label} uploaded the whole file"
        );
    };
    let (hoprs, scp) = alternate(
        cfg,
        "sftp-big",
        || clean(cfg, "sshd", SSH_DIR),
        || {
            let t = rt.block_on(hoprs_sftp(cfg, files, key, files.big, &remote));
            verify("hoprs");
            t
        },
        || {
            let t = run_tool(
                Command::new("scp")
                    .args(["-q", "-B", "-P", &cfg.ssh_port.to_string(), "-i"])
                    .arg(key)
                    .args(ssh_options())
                    .arg(files.local(files.big))
                    .arg(format!("{}@{}:{remote}", cfg.ssh_user, cfg.host)),
            );
            verify("scp");
            t
        },
    );
    let mib = files.big_len as f64 / (1 << 20) as f64;
    let rate = |t: Duration| mib / t.as_secs_f64();
    // Throughput ratio: the inverse of the time ratio.
    let ratio = scp.as_secs_f64() / hoprs.as_secs_f64();
    Outcome {
        target: "SFTP 1 GiB upload ≥ 90% of `scp`",
        rtt_ms: cfg.rtt_ms,
        hoprs: format!("{:.1} MiB/s ({:.1} s)", rate(hoprs), hoprs.as_secs_f64()),
        baseline: format!("`scp`: {:.1} MiB/s ({:.1} s)", rate(scp), scp.as_secs_f64()),
        ratio,
        criterion: "hoprs ≥ 0.9 × `scp` (throughput)",
        pass: ratio >= 0.9,
    }
}

fn sftp_small(cfg: &Config, rt: &Runtime, files: &Files, key: &Path) -> Outcome {
    set_rtt(cfg, "sshd", cfg.small_rtt_ms);
    let remote = format!("{SSH_DIR}/{}", files.tree);
    let connect = format!("ssh -a -x -i {} {}", key.display(), ssh_options().join(" "));
    let script = format!(
        "set sftp:connect-program \"{connect}\"; set net:max-retries 1; \
         open -u {}, sftp://{}:{}; mirror -R --parallel={WORKERS} {} {remote}",
        cfg.ssh_user,
        cfg.host,
        cfg.ssh_port,
        files.local(files.tree).display()
    );
    let (hoprs, lftp) = alternate(
        cfg,
        "sftp-small",
        || clean(cfg, "sshd", SSH_DIR),
        || {
            let t = rt.block_on(hoprs_sftp(cfg, files, key, files.tree, &remote));
            verify_count(cfg, "sshd", &remote, cfg.small_files, "hoprs");
            t
        },
        || {
            let t = run_tool(Command::new("lftp").arg("-c").arg(&script));
            verify_count(cfg, "sshd", &remote, cfg.small_files, "lftp");
            t
        },
    );
    small_outcome(
        cfg,
        "SFTP 10k small files not slower than `lftp`",
        hoprs,
        lftp,
    )
}

fn ftp_small(cfg: &Config, rt: &Runtime, files: &Files) -> Outcome {
    set_rtt(cfg, "ftp", cfg.small_rtt_ms);
    let remote = format!("{FTP_DIR}/{}", files.tree);
    let script = format!(
        "set ftp:ssl-allow no; set net:max-retries 1; open -u {},{} ftp://{}:{}; \
         mirror -R --parallel={WORKERS} {} {remote}",
        cfg.ftp_user,
        cfg.ftp_password,
        cfg.host,
        cfg.ftp_port,
        files.local(files.tree).display()
    );
    let (hoprs, lftp) = alternate(
        cfg,
        "ftp-small",
        || clean(cfg, "ftp", FTP_DIR),
        || {
            let t = rt.block_on(hoprs_ftp(cfg, files, files.tree, &remote));
            verify_count(cfg, "ftp", &remote, cfg.small_files, "hoprs");
            t
        },
        || {
            let t = run_tool(Command::new("lftp").arg("-c").arg(&script));
            verify_count(cfg, "ftp", &remote, cfg.small_files, "lftp");
            t
        },
    );
    small_outcome(
        cfg,
        "FTP 10k small files not slower than `lftp`",
        hoprs,
        lftp,
    )
}

fn small_outcome(cfg: &Config, target: &'static str, hoprs: Duration, lftp: Duration) -> Outcome {
    let ratio = hoprs.as_secs_f64() / lftp.as_secs_f64();
    let rate = |t: Duration| cfg.small_files as f64 / t.as_secs_f64();
    Outcome {
        target,
        rtt_ms: cfg.small_rtt_ms,
        hoprs: format!("{:.1} s ({:.0} files/s)", hoprs.as_secs_f64(), rate(hoprs)),
        baseline: format!(
            "`lftp`: {:.1} s ({:.0} files/s)",
            lftp.as_secs_f64(),
            rate(lftp)
        ),
        ratio,
        criterion: "hoprs ≤ 1.0 × `lftp` (time)",
        pass: ratio <= 1.0,
    }
}

/// Runs both sides `cfg.runs` times, alternating which goes first, cleaning the remote
/// folder before each; returns the median times (hoprs, baseline).
fn alternate(
    cfg: &Config,
    name: &str,
    clean: impl Fn(),
    hoprs: impl Fn() -> Duration,
    baseline: impl Fn() -> Duration,
) -> (Duration, Duration) {
    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    for run in 0..cfg.runs {
        let mut sides: [Side<'_>; 2] = [
            ("hoprs", &hoprs, &mut ours),
            ("baseline", &baseline, &mut theirs),
        ];
        if run % 2 == 1 {
            sides.reverse();
        }
        for (label, side, times) in sides {
            clean();
            let t = side();
            eprintln!("{name} run {}: {label} {:.2} s", run + 1, t.as_secs_f64());
            times.push(t);
        }
    }
    clean();
    (median(ours), median(theirs))
}

/// One side of a run: its label, the run, and where its times go.
type Side<'a> = (&'a str, &'a dyn Fn() -> Duration, &'a mut Vec<Duration>);

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

// ---- hoprs ----------------------------------------------------------------------------

/// Connects (key auth, host key accepted), uploads `local` (under the files' root) to
/// `remote` with 4 workers and waits for the end. The connection counts in the time, as
/// it does for the tools.
async fn hoprs_sftp(
    cfg: &Config,
    files: &Files,
    key: &Path,
    local: &str,
    remote: &str,
) -> Duration {
    let known_hosts = tempfile::tempdir().expect("known_hosts dir");
    let mut ssh = SshConfig::new(cfg.host.clone(), cfg.ssh_port, cfg.ssh_user.clone());
    ssh.known_hosts = known_hosts.path().join("known_hosts");
    ssh.auth.use_agent = false;
    ssh.auth.allow_keyboard_interactive = false;
    ssh.auth.key = KeySource::File {
        path: key.to_path_buf(),
        passphrase: None,
    };
    let start = Instant::now();
    let prompter: Arc<dyn Prompter> = Arc::new(ScriptedPrompter::new().accept_host(true));
    let session = SshSession::connect(ssh, prompter)
        .await
        .unwrap_or_else(|e| fail_setup(&format!("SSH connect: {e}")));
    let fs = SftpFs::new(session.clone(), SftpConfig::default())
        .await
        .unwrap_or_else(|e| fail_setup(&format!("SFTP: {e}")));
    upload(files, Arc::new(fs), local, remote).await;
    let elapsed = start.elapsed();
    session.disconnect().await;
    elapsed
}

/// As [`hoprs_sftp`], over plain FTP with 4 transfer connections.
async fn hoprs_ftp(cfg: &Config, files: &Files, local: &str, remote: &str) -> Duration {
    let mut ftp = FtpConfig::new(
        cfg.host.clone(),
        cfg.ftp_port,
        cfg.ftp_user.clone(),
        FtpSecurity::Plain,
    );
    ftp.transfer_connections = WORKERS;
    let prompter = Arc::new(ScriptedFtpPrompter::new().with_password(Some(&cfg.ftp_password)));
    let start = Instant::now();
    let fs = FtpFs::connect(ftp, prompter)
        .await
        .unwrap_or_else(|e| fail_setup(&format!("FTP connect: {e}")));
    upload(files, Arc::new(fs.clone()), local, remote).await;
    let elapsed = start.elapsed();
    fs.disconnect().await;
    elapsed
}

/// One upload through a fresh engine (queue in memory); panics unless it ends `Done`.
async fn upload(files: &Files, remote_fs: Arc<dyn RemoteFs>, local: &str, remote: &str) {
    let engine = TransferEngine::start(
        QueueStore::open_in_memory().expect("queue"),
        EngineConfig::default(),
    )
    .expect("engine");
    let mut rx = engine.subscribe();
    let site = SiteId("bench".to_owned());
    engine
        .register_site(
            site.clone(),
            SiteEndpoints {
                local: Arc::new(LocalFs::new(&files.root)),
                remote: remote_fs,
                workers: WORKERS,
            },
        )
        .await;
    let id = engine
        .enqueue(TransferSpec {
            site,
            direction: Direction::Upload,
            local: RemotePath::new(&format!("/{local}")).expect("path"),
            remote: RemotePath::new(remote).expect("path"),
        })
        .await
        .expect("enqueue");
    let state = tokio::time::timeout(RUN_TIMEOUT, async {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let EventKind::StateChanged { id: i, state } = event.kind
                        && i == id
                        && state.is_terminal()
                    {
                        return state;
                    }
                }
                // Lost events, maybe the last one: ask the engine.
                Err(RecvError::Lagged(_)) => {
                    if let Some(state) = terminal_state(&engine, id).await {
                        return state;
                    }
                }
                Err(RecvError::Closed) => panic!("the engine stopped"),
            }
        }
    })
    .await
    .expect("the upload finishes in time");
    assert_eq!(state, TransferState::Done, "the upload succeeds");
    engine.shutdown().await;
}

async fn terminal_state(engine: &TransferEngine, id: TransferId) -> Option<TransferState> {
    engine
        .snapshot()
        .await
        .into_iter()
        .find(|item| item.id == id)
        .map(|item| item.state)
        .filter(TransferState::is_terminal)
}

// ---- Tools and containers --------------------------------------------------------------

/// No agent, no known_hosts, never a prompt.
fn ssh_options() -> [&'static str; 8] {
    [
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "BatchMode=yes",
        "-o",
        "LogLevel=ERROR",
    ]
}

/// Runs a baseline tool to completion; its wall time.
fn run_tool(command: &mut Command) -> Duration {
    let program = command.get_program().to_string_lossy().into_owned();
    let start = Instant::now();
    let output = command
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| fail_setup(&format!("cannot run {program}: {e}")));
    let elapsed = start.elapsed();
    if !output.status.success() {
        fail_setup(&format!(
            "{program} failed ({}): {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    elapsed
}

/// `sh -c <script>` in a bench service; its stdout.
fn compose_exec(cfg: &Config, service: &str, script: &str) -> String {
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(&cfg.compose)
        .args(["exec", "-T", service, "sh", "-c", script])
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| fail_setup(&format!("cannot run docker: {e}")));
    if !output.status.success() {
        fail_setup(&format!(
            "`{script}` in {service} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Sets the netem delay on the service's interface: every packet it sends waits `rtt_ms`
/// (one direction delayed by the whole round trip). The queue limit is raised so netem
/// itself never drops packets at these rates.
fn set_rtt(cfg: &Config, service: &str, rtt_ms: u64) {
    let shown = compose_exec(
        cfg,
        service,
        &format!(
            "tc qdisc replace dev eth0 root netem delay {rtt_ms}ms limit 100000 \
             && tc qdisc show dev eth0"
        ),
    );
    assert!(
        shown.contains(&format!("delay {rtt_ms}ms")) || rtt_ms == 0,
        "netem not applied in {service}: {shown}"
    );
    eprintln!("{service}: {}", shown.trim());
}

/// Empties the bench folder of a service.
fn clean(cfg: &Config, service: &str, dir: &str) {
    compose_exec(cfg, service, &format!("find {dir} -mindepth 1 -delete"));
}

fn verify_count(cfg: &Config, service: &str, dir: &str, expected: usize, label: &str) {
    let count = compose_exec(cfg, service, &format!("find {dir} -type f | wc -l"));
    assert_eq!(
        count.trim(),
        expected.to_string(),
        "{label} uploaded every file to {dir}"
    );
}
