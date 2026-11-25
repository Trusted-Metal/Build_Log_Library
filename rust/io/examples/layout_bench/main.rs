//! Layout benchmark for the V18 sensor layout decision (F29 in the live-feed documentation): `build_wide` against `per_layer`,
//! written and read through the library's working-file and seal functions. Phase 1 of `LBL_V18_BENCHMARK_PLAN.md`.
//!
//! ```text
//! cargo run --release -p build-log-io --example layout_bench -- calibrate [--cmd-noise 7,7,1] [--act-noise 60,60,20] [--power-noise 1]
//! cargo run --release -p build-log-io --example layout_bench -- write --layout build_wide|per_layer --path F.lbl [--target-gb 50]
//!        [--lasers 4] [--rate-hz 100000] [--layer-seconds 10] [--sensors 35] [--filters gzip6f32] [--realtime] [--bare] [--no-seal]
//!        [--sensor-chunk 4096] [--max-layers N] [--disorder 0.002] [--templates N]
//! cargo run --release -p build-log-io --example layout_bench -- read --path F.lbl [--repeat 3] [--scan]
//! ```
//!
//! Output: one JSON object per line on stdout (progress on stderr). The data is **synthetic** (see `gen.rs`). Nothing here is
//! part of the library's API.

mod codec;
mod gen;
mod killtest;
mod layout;
mod read;
mod real;

/// The base seed of every synthetic build, so that the kill test can regenerate any layer.
pub const SEED: u64 = 0xC0FFEE;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use build_log_io::container::{create_working_file, seal, SealOptions};
use gen::{
    laser_layer, opcua_layer, LaserLayer, LaserParams, OpcState, Rng, FIXTURE_RATIOS,
    LASER_CHANNELS,
};
use hdf5::File;
use layout::{make, BuildWide, Core, Filters, Layout, PerLayer, Res};
use serde_json::json;

pub struct Args(HashMap<String, String>);

impl Args {
    fn parse(it: impl Iterator<Item = String>) -> Args {
        let mut m = HashMap::new();
        let v: Vec<String> = it.collect();
        let mut i = 0;
        while i < v.len() {
            if let Some(k) = v[i].strip_prefix("--") {
                if i + 1 < v.len() && !v[i + 1].starts_with("--") {
                    m.insert(k.to_string(), v[i + 1].clone());
                    i += 2;
                } else {
                    m.insert(k.to_string(), "true".into());
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        Args(m)
    }
    pub fn get<T: std::str::FromStr>(&self, k: &str, default: T) -> T {
        self.0
            .get(k)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
    pub fn flag(&self, k: &str) -> bool {
        self.0.contains_key(k)
    }
    pub fn text(&self, k: &str) -> Option<String> {
        self.0.get(k).cloned()
    }
}

fn main() -> Res<()> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_default();
    let args = Args::parse(it);
    match cmd.as_str() {
        "calibrate" => calibrate(&args),
        "write" => write(&args),
        "read" => read::run(&args),
        "kill-test" => killtest::kill_test(&args),
        "verify" => killtest::verify_cmd(&args),
        "codec-bench" => codec::bench(&args),
        "seal-kill" => killtest::seal_kill(&args),
        "seal" => killtest::seal_child(&args),
        _ => {
            eprintln!("usage: layout_bench calibrate | write | read   (see the header of layout_bench/main.rs)");
            std::process::exit(2);
        }
    }
}

fn triple(s: &str) -> [i32; 3] {
    let v: Vec<i32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    [v[0], v[1], v[2]]
}

pub(crate) fn laser_params(a: &Args) -> LaserParams {
    let mut p = LaserParams::default();
    if let Some(s) = a.text("cmd-noise") {
        p.cmd_noise = triple(&s);
    }
    if let Some(s) = a.text("act-noise") {
        p.act_noise = triple(&s);
    }
    p.power_noise_lsb = a.get("power-noise", p.power_noise_lsb);
    p
}

/// Writes one fixture-sized laser layer (727,922 samples, chunk 10000, gzip 6) and compares each channel's compression
/// ratio with the fixture's. The generator is honest only if they agree.
fn calibrate(a: &Args) -> Res<()> {
    let p = laser_params(a);
    let dir = std::env::temp_dir().join(format!("layout_bench_cal_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("cal.h5");
    let n = 727_922usize;
    let mut rng = Rng(0x5EED);
    let d = laser_layer(&mut rng, &p, 1, 1, n);
    let f = Filters {
        gzip: 6,
        shuffle: false,
        fletcher32: false,
    };
    let file = File::create(&path)?;
    let mut rows = Vec::new();
    let mut worst: f64 = 0.0;
    for (i, name) in LASER_CHANNELS.iter().enumerate() {
        let (raw, stored) = match i {
            0..=5 => {
                let ds = make::<i32>(&file, name, 10_000, &f, n, false)?;
                ds.write(&d.pos[i])?;
                (n * 4, ds.storage_size())
            }
            6 => {
                let ds = make::<f32>(&file, name, 10_000, &f, n, false)?;
                ds.write(&d.power)?;
                (n * 4, ds.storage_size())
            }
            _ => {
                let ds = make::<u8>(&file, name, 10_000, &f, n, false)?;
                ds.write(&d.state)?;
                (n, ds.storage_size())
            }
        };
        let ratio = raw as f64 / stored.max(1) as f64;
        let err = (ratio / FIXTURE_RATIOS[i] - 1.0) * 100.0;
        // the state channel's ratio is dominated by a few hundred bytes of chunk overhead; judge it loosely
        if i < 7 {
            worst = worst.max(err.abs());
        }
        rows.push(json!({"channel": name, "ratio": (ratio * 100.0).round() / 100.0, "fixture": FIXTURE_RATIOS[i], "error_pct": (err * 10.0).round() / 10.0}));
    }
    drop(file);
    let _ = std::fs::remove_dir_all(&dir);
    println!(
        "{}",
        json!({"what": "calibrate", "params": format!("{p:?}"), "channels": rows, "worst_error_pct_excluding_state": (worst * 10.0).round() / 10.0, "within_25pct": worst <= 25.0})
    );
    Ok(())
}

#[derive(Default)]
struct Lat(Vec<u32>);

impl Lat {
    fn push(&mut self, d: Duration) {
        self.0.push(d.as_micros().min(u32::MAX as u128) as u32);
    }
    fn summary(&self) -> serde_json::Value {
        if self.0.is_empty() {
            return json!({"n": 0});
        }
        let mut v = self.0.clone();
        v.sort_unstable();
        let q = |p: f64| v[(((v.len() - 1) as f64) * p) as usize] as f64 / 1000.0;
        let over = |ms: u32| v.iter().filter(|&&x| x > ms * 1000).count();
        json!({"n": v.len(), "p50_ms": q(0.5), "p99_ms": q(0.99), "p999_ms": q(0.999), "max_ms": *v.last().unwrap() as f64 / 1000.0,
               "over_50ms": over(50), "over_250ms": over(250), "total_s": v.iter().map(|&x| x as f64).sum::<f64>() / 1e6})
    }
    fn p99_ms(&self) -> f64 {
        if self.0.is_empty() {
            return 0.0;
        }
        let mut v = self.0.clone();
        v.sort_unstable();
        v[((v.len() - 1) as f64 * 0.99) as usize] as f64 / 1000.0
    }
}

fn timed<T>(lat: &mut Lat, f: impl FnOnce() -> Res<T>) -> Res<T> {
    let t = Instant::now();
    let r = f()?;
    lat.push(t.elapsed());
    Ok(r)
}

fn write(a: &Args) -> Res<()> {
    let layout_name = a
        .text("layout")
        .ok_or("--layout build_wide|per_layer is required")?;
    let path: PathBuf = a.text("path").ok_or("--path is required")?.into();
    let target_gb: f64 = a.get("target-gb", 1.0);
    let lasers: u32 = a.get("lasers", 4);
    let rate: usize = a.get("rate-hz", 100_000);
    let layer_s: f64 = a.get("layer-seconds", 10.0);
    let sensors: usize = a.get("sensors", 35);
    let filters = Filters::parse(
        &a.text("filters")
            .unwrap_or_else(|| "gzip3shufflef32".into()),
    )?;
    let sensor_chunk: usize = a.get("sensor-chunk", 4096);
    let laser_chunk: usize = a.get("laser-chunk", 10_000);
    // The size of the batches the writer receives (the feed sends 500-sample batches). Datasets are written in whole chunks.
    let block: usize = a.get("batch", 500);
    let max_layers: u32 = a.get("max-layers", u32::MAX);
    let realtime = a.flag("realtime");
    let bare = a.flag("bare");
    let do_seal = !bare && !a.flag("no-seal");
    let templates: usize = a.get("templates", if realtime { 2 } else { 0 });
    let disorder: f64 = a.get("disorder", 0.002);
    // `H5Fflush` every this many layers (0: never). A crash can only be survived up to the last flush.
    let flush_layers: u32 = a.get("flush-layers", 0);
    // build_wide: write the partial chunk of every sensor every this many layers (0: full chunks and `finish` only).
    let tail_layers: u32 = a.get("tail-flush-layers", 0);
    let progress: Option<PathBuf> = a.text("progress").map(PathBuf::from);
    let params = laser_params(a);
    // F31: by default a chunk cache of two chunks per dataset; `--libhdf5-cache` keeps libhdf5's own 1 MiB for comparison.
    layout::CHUNK_CACHE_BYTES.store(
        if a.flag("libhdf5-cache") {
            -2
        } else {
            a.get::<i64>("chunk-cache-kb", -1)
                .saturating_mul(1024)
                .max(-1)
        },
        std::sync::atomic::Ordering::Relaxed,
    );

    let n = (rate as f64 * layer_s) as usize / block * block;

    let file = if bare {
        File::create_excl(&path)?
    } else {
        create_working_file(&path)?
    };
    let mut core = Core::new(file, lasers, filters, laser_chunk, rate)?;
    core.encode = a.flag("encode");
    let mut lay: Box<dyn Layout> = match layout_name.as_str() {
        "build_wide" => {
            let mut w = BuildWide::new(core, sensors, sensor_chunk)?;
            w.tail_every = tail_layers;
            Box::new(w)
        }
        "per_layer" => Box::new(PerLayer::new(core, sensors, sensor_chunk)?),
        other => return Err(format!("unknown layout '{other}'").into()),
    };

    let mut opc = OpcState::new(sensors, disorder);
    // Real layers (plan step 16): tiled from a V17 build; they vary in length, so the loop below takes each layer's own length.
    let real_pool: Vec<Vec<LaserLayer>> = match a.text("real") {
        Some(p) => real::load(&p, lasers, a.get("real-layers", 40), 2 * laser_chunk)?,
        None => Vec::new(),
    };
    if !real_pool.is_empty() {
        let lens: Vec<usize> = real_pool.iter().map(|s| s[0].power.len()).collect();
        eprintln!(
            "real pool: {} layers, {} to {} samples (median {}), tiled",
            lens.len(),
            lens.iter().min().unwrap(),
            lens.iter().max().unwrap(),
            {
                let mut s = lens.clone();
                s.sort_unstable();
                s[s.len() / 2]
            }
        );
    }
    let mut pool: Vec<Vec<LaserLayer>> = Vec::new();
    for t in 0..templates {
        pool.push(
            (1..=lasers)
                .map(|l| {
                    laser_layer(
                        &mut Rng::seeded(SEED, t as u32 + 1, l),
                        &params,
                        l,
                        t as u32 + 1,
                        n,
                    )
                })
                .collect(),
        );
    }

    let (mut l_laser, mut l_opc, mut l_begin, mut l_end, mut l_flush) = (
        Lat::default(),
        Lat::default(),
        Lat::default(),
        Lat::default(),
        Lat::default(),
    );
    let (mut w_laser, mut w_opc) = (Lat::default(), Lat::default()); // the current window, for the trend over depth
    let mut trend = Vec::new();
    let mut raw_bytes: u64 = 0;
    let mut gen_time = Duration::ZERO;
    let mut max_lag = Duration::ZERO;
    let started = Instant::now();
    let target_bytes = (target_gb * 1e9) as u64;
    let mut layers = 0u32;
    let mut build_ns = 0u64; // the time of the build so far, by the laser clock

    loop {
        layers += 1;
        let layer = layers;
        let tg = Instant::now();
        let fresh: Vec<LaserLayer>;
        let data: &Vec<LaserLayer> = if !real_pool.is_empty() {
            &real_pool[(layer as usize - 1) % real_pool.len()]
        } else if templates > 0 {
            &pool[(layer as usize - 1) % templates]
        } else {
            fresh = (1..=lasers)
                .map(|l| laser_layer(&mut Rng::seeded(SEED, layer, l), &params, l, layer, n))
                .collect();
            &fresh
        };
        let n = data[0].power.len(); // this layer's samples (a real layer has its own length)
        let layer_ns = (n as u64 * 1_000_000_000) / rate as u64;
        let start_ns = build_ns;
        let recs = opcua_layer(
            &mut Rng::seeded(SEED, layer, 99),
            &mut opc,
            layer,
            start_ns,
            layer_ns,
        );
        gen_time += tg.elapsed();

        timed(&mut l_begin, || lay.core().begin_layer(layer))?;
        let mut next = 0usize;
        for b in 0..n.div_ceil(block) {
            let (lo, hi) = (b * block, ((b + 1) * block).min(n));
            for l in 1..=lasers {
                let t = Instant::now();
                lay.core().append_laser(l, &data[l as usize - 1], lo, hi)?;
                let d = t.elapsed();
                l_laser.push(d);
                w_laser.push(d);
            }
            raw_bytes += (lasers as usize * (hi - lo) * 29) as u64;
            let slice_end = start_ns + (hi as u64 * 1_000_000_000) / rate as u64;
            let from = next;
            while next < recs.len() && recs[next].arrival_ns < slice_end {
                next += 1;
            }
            if next > from {
                let t = Instant::now();
                lay.append_opcua(layer, &recs[from..next])?;
                let d = t.elapsed();
                l_opc.push(d);
                w_opc.push(d);
                raw_bytes += ((next - from) * 20) as u64;
            }
            if let Some(p) = &progress {
                // What the writer has been handed so far, for the crash test (a torn write of this file is tolerated).
                let _ = std::fs::write(p, format!("{layer} {b}"));
            }
            if realtime {
                let due = started + Duration::from_nanos(slice_end);
                let now = Instant::now();
                if now < due {
                    std::thread::sleep(due - now);
                } else {
                    max_lag = max_lag.max(now - due);
                }
            }
        }
        if next < recs.len() {
            let t = Instant::now();
            lay.append_opcua(layer, &recs[next..])?;
            l_opc.push(t.elapsed());
            raw_bytes += ((recs.len() - next) * 20) as u64;
        }
        timed(&mut l_end, || {
            lay.core().end_layer(layer, n, data)?;
            lay.end_layer(layer)
        })?;
        build_ns += layer_ns;
        if flush_layers > 0 && layer % flush_layers == 0 {
            timed(&mut l_flush, || lay.core().flush_file())?;
        }

        let size = lay.core().file.size();
        if layers % 50 == 0 {
            trend.push(json!({"layer": layers, "file_gb": size as f64 / 1e9, "elapsed_s": started.elapsed().as_secs(),
                              "laser_p99_ms": w_laser.p99_ms(), "opcua_p99_ms": w_opc.p99_ms()}));
            eprintln!(
                "layer {layers}: {:.2} GB, {}s, laser p99 {:.1} ms, opcua p99 {:.1} ms",
                size as f64 / 1e9,
                started.elapsed().as_secs(),
                w_laser.p99_ms(),
                w_opc.p99_ms()
            );
            w_laser = Lat::default();
            w_opc = Lat::default();
        }
        if size >= target_bytes || layers >= max_layers {
            break;
        }
    }

    let write_elapsed = started.elapsed();
    let tf = Instant::now();
    let stages = lay.finish(layers)?;
    let finish_s = tf.elapsed().as_secs_f64();
    let h5_calls_s: f64 = [&l_laser, &l_opc, &l_begin, &l_end, &l_flush]
        .iter()
        .map(|l| l.0.iter().map(|&x| x as f64).sum::<f64>() / 1e6)
        .sum();
    let file_bytes = lay.core().file.size();
    let encode_s = lay.core().encode_ns as f64 / 1e9;
    drop(lay);

    let mut seal_json = json!(null);
    if do_seal {
        let ts = Instant::now();
        let report = seal(&path, &SealOptions::new())?;
        seal_json = json!({"seconds": ts.elapsed().as_secs_f64(), "archive_bytes": report.archive_len, "hdf5_bytes": report.hdf5_len});
    }
    let final_bytes = std::fs::metadata(&path)?.len();

    println!(
        "{}",
        json!({
            "what": "write", "layout": layout_name, "path": path.display().to_string(), "bare": bare, "sealed": do_seal,
            "filters": format!("{filters:?}"), "realtime": realtime, "templates": templates, "synthetic": true,
            "layers": layers, "lasers": lasers, "rate_hz": rate, "layer_seconds": layer_s, "sensors": sensors, "sensor_chunk": sensor_chunk,
            "file_bytes_before_seal": file_bytes, "file_bytes_final": final_bytes,
            "raw_bytes": raw_bytes, "compression_ratio": raw_bytes as f64 / file_bytes as f64,
            "wall_s": write_elapsed.as_secs_f64(), "generator_s": gen_time.as_secs_f64(), "h5_calls_s": h5_calls_s,
            "raw_mb_s_in_h5_calls": raw_bytes as f64 / 1e6 / h5_calls_s,
            "realtime_factor_headroom": (layers as f64 * layer_s) / h5_calls_s,
            "max_lag_ms": max_lag.as_secs_f64() * 1000.0,
            "latency": {"laser_block": l_laser.summary(), "opcua_append": l_opc.summary(), "layer_begin": l_begin.summary(), "layer_end": l_end.summary(), "file_flush": l_flush.summary()},
            "flush_layers": flush_layers, "tail_flush_layers": tail_layers, "batch": block,
            "encode": a.flag("encode"), "encode_s": encode_s,
            "finish_s": finish_s, "finish_stages": stages.into_iter().map(|(k, v)| json!({k: v})).collect::<Vec<_>>(),
            "seal": seal_json, "trend": trend,
        })
    );
    Ok(())
}
