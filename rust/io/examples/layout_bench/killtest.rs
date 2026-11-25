//! W10: kill the writer at random moments and judge what is left on disk.
//!
//! `kill-test` starts the writer as a child process, kills it (on Windows `TerminateProcess`, the equivalent of `kill -9`: no
//! flush, no close) after a random time, then opens what is left and compares it with what the generator says should be
//! there. The generator is deterministic per layer (see `gen.rs`), so the expected content of any layer can be regenerated.
//!
//! What it distinguishes, from least to most serious:
//!   * data that was handed to the writer but is not on disk (expected: a crash loses what was not written or flushed);
//!   * a file that does not open (it must be reported as unusable, never as a different file);
//!   * a layer whose channels have different lengths (torn: must be cut back to the shortest before use);
//!   * **silent corruption**: data that reads without error but differs from what was written. This is the tripwire.
//!
//! A process kill does not lose what the operating system already holds, so this is not a power-loss test (S10).
//! `seal-kill` does the same for the seal: it must leave a working file that can be sealed again, never a damaged archive.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use build_log_io::container::{seal, FileArchive, SealOptions};
use hdf5::types::VarLenUnicode;
use hdf5::File;
use serde_json::{json, Value};

use crate::gen::{laser_layer, opcua_layer, sensor_names, LaserLayer, LaserParams, OpcState, Rng};
use crate::layout::Res;
use crate::{laser_params, Args, SEED};

struct Cfg {
    lasers: u32,
    sensors: usize,
    rate: usize,
    layer_s: f64,
    batch: usize,
    disorder: f64,
    sensor_chunk: usize,
    params: LaserParams,
}

impl Cfg {
    fn n(&self) -> usize {
        (self.rate as f64 * self.layer_s) as usize / self.batch * self.batch
    }
    fn layer_ns(&self) -> u64 {
        (self.n() as u64 * 1_000_000_000) / self.rate as u64
    }
    fn block_ns(&self) -> u64 {
        (self.batch as u64 * 1_000_000_000) / self.rate as u64
    }
}

fn short(e: impl std::fmt::Display) -> String {
    let s = e.to_string().replace('\n', " ");
    s.chars().take(160).collect()
}

fn read_progress(p: &Path) -> Option<(u32, usize)> {
    let s = std::fs::read_to_string(p).ok()?;
    let mut it = s.split_whitespace();
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// `verify --path F [--layer-seconds 3] [--progress P]`: judges one file as the kill test does. Also how the verifier itself
/// is tested: a deliberately tampered file must be reported as silently corrupt.
pub fn verify_cmd(a: &Args) -> Res<()> {
    let path: PathBuf = a.text("path").ok_or("--path is required")?.into();
    let c = Cfg {
        lasers: a.get("lasers", 4),
        sensors: a.get("sensors", 35),
        rate: a.get("rate-hz", 100_000),
        layer_s: a.get("layer-seconds", 3.0),
        batch: a.get("batch", 500),
        disorder: a.get("disorder", 0.002),
        sensor_chunk: a.get("sensor-chunk", 4096),
        params: laser_params(a),
    };
    let fed = a
        .text("progress")
        .and_then(|p| read_progress(Path::new(&p)));
    println!("{}", verify(&path, &c, fed));
    Ok(())
}

pub fn kill_test(a: &Args) -> Res<()> {
    let runs: usize = a.get("runs", 40);
    let layout = a.text("layout").ok_or("--layout is required")?;
    let flush_layers: u32 = a.get("flush-layers", 0);
    let tail_layers: u32 = a.get("tail-flush-layers", 0);
    let min_s: f64 = a.get("min-s", 2.0);
    let max_s: f64 = a.get("max-s", 20.0);
    // A control: the writer runs 3 layers and finishes normally. Nothing may be lost or flagged, or the verifier is wrong.
    let control = a.flag("control");
    let layer_s: f64 = a.get("layer-seconds", 3.0);
    let dir: PathBuf = a
        .text("dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("layout_bench_kill"));
    let c = Cfg {
        lasers: a.get("lasers", 4),
        sensors: a.get("sensors", 35),
        rate: a.get("rate-hz", 100_000),
        layer_s,
        batch: a.get("batch", 500),
        disorder: a.get("disorder", 0.002),
        sensor_chunk: a.get("sensor-chunk", 4096),
        params: laser_params(a),
    };
    std::fs::create_dir_all(&dir)?;
    let exe = std::env::current_exe()?;
    let mut rng = Rng(a.get("seed", 7u64) | 1);
    let label = format!("{layout}/flush{flush_layers}/tail{tail_layers}");
    let mut results: Vec<Value> = Vec::new();

    for i in 0..runs {
        let path = dir.join(format!("kill_{i}.lbl"));
        let prog = dir.join(format!("kill_{i}.progress"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&prog);
        let mut child = Command::new(&exe)
            .args(["write", "--layout", &layout, "--path"])
            .arg(&path)
            .args([
                "--target-gb",
                "9999",
                "--no-seal",
                "--layer-seconds",
                &layer_s.to_string(),
                "--flush-layers",
                &flush_layers.to_string(),
                "--tail-flush-layers",
                &tail_layers.to_string(),
                "--disorder",
                &c.disorder.to_string(),
                "--sensors",
                &c.sensors.to_string(),
                "--lasers",
                &c.lasers.to_string(),
                "--max-layers",
                &(if control { 3u32 } else { u32::MAX }).to_string(),
                "--progress",
            ])
            .arg(&prog)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let wait = min_s + rng.unit() * (max_s - min_s);
        let exited_early = if control {
            let _ = child.wait();
            false
        } else {
            std::thread::sleep(Duration::from_secs_f64(wait));
            let early = matches!(child.try_wait(), Ok(Some(_)));
            let _ = child.kill();
            let _ = child.wait();
            early
        };

        let fed = read_progress(&prog);
        let t = Instant::now();
        let mut v = verify(&path, &c, fed);
        v["run"] = json!(i);
        v["killed_after_s"] = json!((wait * 10.0).round() / 10.0);
        v["child_exited_early"] = json!(exited_early);
        v["file_gb"] = json!(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as f64 / 1e9);
        v["verify_s"] = json!(t.elapsed().as_secs_f64());
        eprintln!("{label} run {i}: {}", short(summary_line(&v)));
        results.push(v);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&prog);
    }
    println!("{}", summarize(&label, &results));
    for r in &results {
        println!("{r}");
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn summary_line(v: &Value) -> String {
    format!(
        "opened={} silent={} torn={} layers={} lost_laser_ms={} lost_opcua_rows={}",
        v["opened"],
        v["silent_corruption_count"],
        v["torn_layer_lasers"],
        v["layers_found"],
        v["lost_laser_ms"],
        v["opcua_rows_lost"]
    )
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn summarize(label: &str, rs: &[Value]) -> Value {
    let opened = rs.iter().filter(|r| r["opened"] == json!(true)).count();
    let silent = rs
        .iter()
        .filter(|r| r["silent_corruption_count"].as_u64().unwrap_or(0) > 0)
        .count();
    let torn = rs
        .iter()
        .filter(|r| r["torn_layer_lasers"].as_u64().unwrap_or(0) > 0)
        .count();
    let issues = rs
        .iter()
        .filter(|r| r["opcua_structural_issues"].as_u64().unwrap_or(0) > 0)
        .count();
    let lost: Vec<f64> = rs
        .iter()
        .filter_map(|r| r["lost_laser_ms"].as_f64())
        .collect();
    let lost_rows: Vec<f64> = rs
        .iter()
        .filter_map(|r| r["opcua_rows_lost"].as_f64())
        .collect();
    json!({"what": "W10_kill_summary", "config": label, "runs": rs.len(), "opened": opened, "did_not_open": rs.len() - opened,
           "runs_with_silent_corruption": silent, "runs_with_torn_layers": torn, "runs_with_opcua_structural_issues": issues,
           "lost_laser_ms": {"median": median(lost.clone()), "max": lost.iter().cloned().fold(0.0, f64::max)},
           "opcua_rows_lost": {"median": median(lost_rows.clone()), "max": lost_rows.iter().cloned().fold(0.0, f64::max)}})
}

#[derive(Default)]
struct Rep {
    layers_found: u64,
    layers_complete: u64,
    layer_lasers_partial: u64,
    torn_layer_lasers: u64,
    recoverable_samples: u64,
    read_errors: u64,
    silent: Vec<String>,
    notes: Vec<String>,
    opcua_rows: u64,
    opcua_issues: u64,
}

fn try_open(path: &Path) -> (Option<File>, Option<String>, bool) {
    match File::open(path) {
        Ok(f) => (Some(f), None, false),
        Err(e) => {
            // Best effort: HDF5's own tool for files left with status flags set.
            let tool = std::env::var("H5CLEAR").unwrap_or_else(|_| {
                r"C:\Program Files\HDF_Group\HDF5\1.14.5\bin\h5clear.exe".into()
            });
            if Path::new(&tool).exists() {
                let _ = Command::new(&tool)
                    .args(["-s", "-m"])
                    .arg(path)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                if let Ok(f) = File::open(path) {
                    return (Some(f), Some(short(e)), true);
                }
            }
            (None, Some(short(e)), false)
        }
    }
}

fn verify(path: &Path, c: &Cfg, fed: Option<(u32, usize)>) -> Value {
    let (file, err, by_h5clear) = try_open(path);
    let Some(file) = file else {
        return json!({"opened": false, "error": err, "fed": fed.map(|f| json!({"layer": f.0, "batch": f.1}))});
    };
    let mut r = Rep::default();
    let mut layout = String::from("?");
    let outcome = walk(&file, c, fed, &mut r, &mut layout);
    if let Err(e) = outcome {
        r.notes.push(format!("walk stopped: {}", short(e)));
    }
    let n = c.n();
    let (fed_samples, lost_ms) = match fed {
        Some((l, b)) => {
            let s = (l as usize - 1) * n + (b + 1) * c.batch;
            let rec = r.recoverable_samples as f64 / c.lasers as f64;
            (
                Some(s),
                Some(((s as f64 - rec).max(0.0) / c.rate as f64 * 1000.0 * 10.0).round() / 10.0),
            )
        }
        None => (None, None),
    };
    let expected_rows = fed.map(|f| expected_fed_rows(c, f)).unwrap_or(0);
    json!({"opened": true, "opened_after_h5clear": by_h5clear, "open_error_before_h5clear": err, "layout": layout,
           "fed": fed.map(|f| json!({"layer": f.0, "batch": f.1})), "fed_samples_per_laser": fed_samples,
           "layers_found": r.layers_found, "layers_complete": r.layers_complete,
           "layer_lasers_partial_consistent": r.layer_lasers_partial, "torn_layer_lasers": r.torn_layer_lasers,
           "recoverable_samples_all_lasers": r.recoverable_samples, "lost_laser_ms": lost_ms,
           "read_errors": r.read_errors, "silent_corruption_count": r.silent.len(), "silent_corruption_examples": r.silent.iter().take(3).collect::<Vec<_>>(),
           "opcua_rows_on_disk": r.opcua_rows, "opcua_rows_fed": expected_rows, "opcua_rows_lost": expected_rows.saturating_sub(r.opcua_rows),
           "opcua_structural_issues": r.opcua_issues, "notes": r.notes.iter().take(5).collect::<Vec<_>>()})
}

fn expected_fed_rows(c: &Cfg, f: (u32, usize)) -> u64 {
    let (fl, fb) = f;
    let mut total = 0u64;
    let mut st = OpcState::new(c.sensors, c.disorder);
    for layer in 1..=fl {
        let start = (layer as u64 - 1) * c.layer_ns();
        let recs = opcua_layer(
            &mut Rng::seeded(SEED, layer, 99),
            &mut st,
            layer,
            start,
            c.layer_ns(),
        );
        if layer < fl {
            total += recs.len() as u64;
        } else {
            let cut = start + (fb as u64 + 1) * c.block_ns();
            total += recs.iter().filter(|r| r.arrival_ns < cut).count() as u64;
        }
    }
    total
}

fn walk(
    file: &File,
    c: &Cfg,
    fed: Option<(u32, usize)>,
    r: &mut Rep,
    layout: &mut String,
) -> Res<()> {
    *layout = file
        .group("Build")?
        .attr("Sensor_Layout")?
        .read_scalar::<VarLenUnicode>()?
        .as_str()
        .to_string();
    let n = c.n();
    let mut layers: Vec<u32> = file
        .group("Build/Lasers/Laser_1/Monitoring_Data")?
        .member_names()?
        .iter()
        .filter_map(|s| s.strip_prefix("Layer_")?.parse().ok())
        .collect();
    layers.sort_unstable();
    r.layers_found = layers.len() as u64;
    let chans = crate::gen::LASER_CHANNELS;

    for &layer in &layers {
        let mut complete = true;
        for l in 1..=c.lasers {
            let expected: LaserLayer =
                laser_layer(&mut Rng::seeded(SEED, layer, l), &c.params, l, layer, n);
            let Ok(g) = file.group(&format!(
                "Build/Lasers/Laser_{l}/Monitoring_Data/Layer_{layer}"
            )) else {
                r.torn_layer_lasers += 1;
                complete = false;
                r.notes
                    .push(format!("layer {layer} laser {l}: group missing"));
                continue;
            };
            let mut lens = [0usize; 8];
            for (k, name) in chans.iter().enumerate() {
                let res: Result<usize, String> = (|| {
                    let ds = g.dataset(name).map_err(short)?;
                    match k {
                        0..=5 => {
                            let v = ds.read_raw::<i32>().map_err(short)?;
                            if v[..] != expected.pos[k][..v.len().min(n)] || v.len() > n {
                                return Err("MISMATCH".into());
                            }
                            Ok(v.len())
                        }
                        6 => {
                            let v = ds.read_raw::<f32>().map_err(short)?;
                            let same = v.len() <= n
                                && v.iter()
                                    .zip(&expected.power)
                                    .all(|(a, b)| a.to_bits() == b.to_bits());
                            if !same {
                                return Err("MISMATCH".into());
                            }
                            Ok(v.len())
                        }
                        _ => {
                            let v = ds.read_raw::<u8>().map_err(short)?;
                            if v.len() > n || v[..] != expected.state[..v.len()] {
                                return Err("MISMATCH".into());
                            }
                            Ok(v.len())
                        }
                    }
                })();
                match res {
                    Ok(len) => lens[k] = len,
                    Err(e) if e == "MISMATCH" => r
                        .silent
                        .push(format!("layer {layer} laser {l} {name}: reads without error but differs from what was written")),
                    Err(e) => {
                        r.read_errors += 1;
                        r.notes.push(format!("layer {layer} laser {l} {name}: {e}"));
                    }
                }
            }
            let min = *lens.iter().min().unwrap();
            let max = *lens.iter().max().unwrap();
            r.recoverable_samples += min as u64;
            if min == n && max == n {
                // whole layer for this laser
            } else if min == max {
                r.layer_lasers_partial += 1;
                complete = false;
            } else {
                r.torn_layer_lasers += 1;
                complete = false;
            }
        }
        if complete {
            r.layers_complete += 1;
        }
    }
    opcua(file, c, fed, r, layout, &layers)
}

fn opcua(
    file: &File,
    c: &Cfg,
    _fed: Option<(u32, usize)>,
    r: &mut Rep,
    layout: &str,
    layers: &[u32],
) -> Res<()> {
    let names = sensor_names(c.sensors);
    let mut st = OpcState::new(c.sensors, c.disorder);
    let max_layer = layers.iter().copied().max().unwrap_or(0);
    // Expected records of each layer, by sensor.
    let mut expected: Vec<Vec<Vec<(u64, u64)>>> = Vec::new(); // [layer-1][sensor] -> (ts, value bits)
    for layer in 1..=max_layer.max(1) {
        let start = (layer as u64 - 1) * c.layer_ns();
        let recs = opcua_layer(
            &mut Rng::seeded(SEED, layer, 99),
            &mut st,
            layer,
            start,
            c.layer_ns(),
        );
        let mut per: Vec<Vec<(u64, u64)>> = vec![Vec::new(); c.sensors];
        for rec in recs {
            per[rec.sensor as usize].push((rec.ts, rec.value.to_bits()));
        }
        expected.push(per);
    }
    if layout == "build_wide" {
        let sets: Vec<Vec<HashSet<(u64, u64)>>> = expected
            .iter()
            .map(|per| per.iter().map(|v| v.iter().copied().collect()).collect())
            .collect();
        for (s, name) in names.iter().enumerate() {
            let Ok(g) = file.group(&format!("Build/Sensors/{name}/Monitoring_Data")) else {
                continue;
            };
            let ts = g.dataset("Source_Ts")?.read_raw::<u64>()?;
            let val = g.dataset("Value")?.read_raw::<f64>()?;
            let lay = g.dataset("Layer")?.read_raw::<u32>()?;
            let idx = g.dataset("Ts_Index")?.read_raw::<u64>()?;
            if !(ts.len() == val.len() && val.len() == lay.len()) {
                r.opcua_issues += 1;
                r.notes.push(format!(
                    "sensor {s}: columns differ in length ({} {} {})",
                    ts.len(),
                    val.len(),
                    lay.len()
                ));
            }
            let rows = ts.len().min(val.len()).min(lay.len());
            r.opcua_rows += rows as u64;
            for k in 0..rows {
                let l = lay[k] as usize;
                let ok = l >= 1
                    && l <= sets.len()
                    && sets[l - 1][s].contains(&(ts[k], val[k].to_bits()));
                if !ok {
                    r.silent.push(format!(
                        "sensor {s} row {k}: a record that was never written"
                    ));
                    break;
                }
            }
            let want: Vec<u64> = ts.iter().step_by(c.sensor_chunk).copied().collect();
            if want != idx {
                r.opcua_issues += 1;
                r.notes.push(format!(
                    "sensor {s}: chunk index has {} entries, the data needs {} (a query would be wrong or miss rows)",
                    idx.len(),
                    want.len()
                ));
            }
        }
    } else {
        for layer in 1..=max_layer {
            for (s, name) in names.iter().enumerate() {
                let Ok(g) = file.group(&format!(
                    "Build/Sensors/{name}/Monitoring_Data/Layer_{layer}"
                )) else {
                    continue;
                };
                let ts = g.dataset("Source_Ts")?.read_raw::<u64>()?;
                let val = g.dataset("Value")?.read_raw::<f64>()?;
                let mut want = expected[layer as usize - 1][s].clone();
                want.sort_by_key(|x| x.0);
                let got: Vec<(u64, u64)> = ts
                    .iter()
                    .zip(&val)
                    .map(|(t, v)| (*t, v.to_bits()))
                    .collect();
                r.opcua_rows += got.len() as u64;
                if got != want {
                    if ts.iter().all(|&t| t == 0) {
                        r.silent.push(format!("layer {layer} sensor {s}: the dataset exists and reads as zeros (created, never written)"));
                    } else {
                        r.silent.push(format!(
                            "layer {layer} sensor {s}: contents differ from what was written"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

// ---- killing the seal ------------------------------------------------------------------------------------------------

/// The child of `seal-kill`: seals a working file and exits.
pub fn seal_child(a: &Args) -> Res<()> {
    let path = a.text("path").ok_or("--path is required")?;
    seal(Path::new(&path), &SealOptions::new())?;
    Ok(())
}

pub fn seal_kill(a: &Args) -> Res<()> {
    let runs: usize = a.get("runs", 20);
    let base_gb: f64 = a.get("base-gb", 2.0);
    let dir: PathBuf = a
        .text("dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("layout_bench_sealkill"));
    std::fs::create_dir_all(&dir)?;
    let exe = std::env::current_exe()?;
    let base = dir.join("base.lbl");
    let _ = std::fs::remove_file(&base);
    eprintln!("writing a {base_gb} GB working file to seal ...");
    let st = Command::new(&exe)
        .args(["write", "--layout", "build_wide", "--path"])
        .arg(&base)
        .args(["--target-gb", &base_gb.to_string(), "--no-seal"])
        .stdout(Stdio::null())
        .status()?;
    if !st.success() {
        return Err("could not write the base file".into());
    }
    // How long does an uninterrupted seal take? Kills are spread over that time.
    let probe = dir.join("probe.lbl");
    std::fs::copy(&base, &probe)?;
    let t = Instant::now();
    Command::new(&exe)
        .args(["seal", "--path"])
        .arg(&probe)
        .stdout(Stdio::null())
        .status()?;
    let seal_s = t.elapsed().as_secs_f64();
    let _ = std::fs::remove_file(&probe);
    eprintln!("an uninterrupted seal (process start to exit) takes {seal_s:.2} s");

    let mut rng = Rng(a.get("seed", 11u64) | 1);
    let (mut archive_complete, mut resealed_ok, mut damaged) = (0u32, 0u32, 0u32);
    let mut details = Vec::new();
    for i in 0..runs {
        let p = dir.join(format!("run_{i}.lbl"));
        let _ = std::fs::remove_file(&p);
        std::fs::copy(&base, &p)?;
        let mut child = Command::new(&exe)
            .args(["seal", "--path"])
            .arg(&p)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let wait = rng.unit() * seal_s * 0.95;
        std::thread::sleep(Duration::from_secs_f64(wait));
        let finished = matches!(child.try_wait(), Ok(Some(_)));
        let _ = child.kill();
        let _ = child.wait();

        // What a restart sees: is it a finished archive, or a working file?
        let mut state = String::new();
        let class = match FileArchive::open_path(&p) {
            Ok(arc) if arc.verify(arc.primary()).is_ok() => {
                archive_complete += 1;
                state.push_str("archive complete and its CRC verifies");
                "archive_complete"
            }
            _ => {
                // Not an archive: it must still be a working file that opens and seals again.
                let opens = File::open(&p).is_ok();
                let reseal = seal(&p, &SealOptions::new());
                let ok = match (&reseal, FileArchive::open_path(&p)) {
                    (Ok(_), Ok(arc)) => arc.verify(arc.primary()).is_ok(),
                    _ => false,
                };
                state.push_str(&format!(
                    "not an archive; opens as HDF5: {opens}; sealing again: {}; verifies: {ok}",
                    match &reseal {
                        Ok(_) => "ok".to_string(),
                        Err(e) => short(e),
                    }
                ));
                if opens && ok {
                    resealed_ok += 1;
                    "working_file_resealed_ok"
                } else {
                    damaged += 1;
                    "DAMAGED"
                }
            }
        };
        eprintln!("seal-kill run {i}: killed at {wait:.2}s (finished: {finished}): {class}");
        details.push(json!({"run": i, "killed_after_s": (wait * 100.0).round() / 100.0, "finished_before_kill": finished, "class": class, "state": state}));
        let _ = std::fs::remove_file(&p);
    }
    println!(
        "{}",
        json!({"what": "W10b_seal_kill_summary", "runs": runs, "base_gb": base_gb, "seal_s": seal_s,
               "archive_complete": archive_complete, "working_file_resealed_ok": resealed_ok, "damaged": damaged})
    );
    for d in details {
        println!("{d}");
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
