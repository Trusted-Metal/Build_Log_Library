//! The read side: the queries of plan items R1, R2, R4, R5, R6, R7, run against a file written by `write`. It opens the file
//! with the plain HDF5 binding, so it reads a sealed `.lbl` (the archive's own file) and a bare `.h5` the same way.

use std::time::Instant;

use hdf5::types::VarLenUnicode;
use hdf5::File;
use serde_json::{json, Value};

use crate::gen::LASER_CHANNELS;
use crate::layout::Res;
use crate::Args;

/// Runs `f` `repeat` times. Returns the first run (the closest to cold that a repeat run can be), the best of the rest, and
/// the result of the last run.
fn runs<T>(repeat: usize, mut f: impl FnMut() -> Res<T>) -> Res<(f64, f64, T)> {
    let t = Instant::now();
    let mut out = f()?;
    let first = t.elapsed().as_secs_f64() * 1000.0;
    let mut best = f64::MAX;
    for _ in 1..repeat.max(1) {
        let t = Instant::now();
        out = f()?;
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
    }
    Ok((first, if best == f64::MAX { first } else { best }, out))
}

fn rec(what: &str, extra: Value, first: f64, best: f64, rows: u64) -> Value {
    json!({"what": what, "params": extra, "first_ms": (first * 10.0).round() / 10.0, "best_of_rest_ms": (best * 10.0).round() / 10.0, "rows": rows})
}

struct Ctx {
    file: File,
    build_wide: bool,
    sensors: Vec<String>,
    layers: u32,
    chunk: usize,
}

pub fn run(a: &Args) -> Res<()> {
    let path = a.text("path").ok_or("--path is required")?;
    let repeat: usize = a.get("repeat", 3);
    let sensor_chunk: usize = a.get("sensor-chunk", 4096);

    let t = Instant::now();
    let file = File::open(&path)?;
    let open_ms = t.elapsed().as_secs_f64() * 1000.0;
    let layout: String = file
        .group("Build")?
        .attr("Sensor_Layout")?
        .read_scalar::<VarLenUnicode>()?
        .as_str()
        .to_string();
    let mut sensors = file.group("Build/Sensors")?.member_names()?;
    sensors.sort();
    let layers = file
        .group("Build/Lasers/Laser_1/Monitoring_Data")?
        .member_names()?
        .len() as u32;
    let lasers = file.group("Build/Lasers")?.member_names()?.len() as u32;
    let cx = Ctx {
        build_wide: layout == "build_wide",
        file,
        sensors,
        layers,
        chunk: sensor_chunk,
    };
    println!(
        "{}",
        json!({"what": "open", "path": path, "layout": layout, "file_bytes": std::fs::metadata(&path)?.len(), "open_ms": open_ms,
               "layers": layers, "lasers": lasers, "sensors": cx.sensors.len()})
    );

    // The OPC UA time span of the build, from the indexes (not part of the timings).
    let (t0, t1) = span(&cx)?;
    println!(
        "{}",
        json!({"what": "span", "opcua_ts_ns": [t0, t1], "seconds": (t1 - t0) as f64 / 1e9})
    );

    // R2: a time window across every sensor, at the start, middle and end of the build.
    for (label, secs) in [("1s", 1u64), ("1min", 60), ("1h", 3600), ("1day", 86_400)] {
        let win = (secs * 1_000_000_000).min(t1 - t0);
        for (pos, frac) in [("start", 0.0f64), ("middle", 0.5), ("end", 1.0)] {
            let lo = t0 + (((t1 - t0 - win) as f64) * frac) as u64;
            let (first, best, rows) = runs(repeat, || window(&cx, lo, lo + win))?;
            println!(
                "{}",
                rec(
                    "R2_window_all_sensors",
                    json!({"window": label, "at": pos, "layout": layout}),
                    first,
                    best,
                    rows
                )
            );
        }
    }

    // R4: one sensor over the whole build.
    let (first, best, rows) = runs(repeat, || one_sensor(&cx, 0))?;
    println!(
        "{}",
        rec(
            "R4_one_sensor_whole_build",
            json!({"layout": layout}),
            first,
            best,
            rows
        )
    );

    // R5: one layer, everything (all lasers and channels, all sensors).
    for (pos, l) in [
        ("first", 1u32),
        ("middle", (layers / 2).max(1)),
        ("last", layers),
    ] {
        let (first, best, (laser_rows, sensor_rows)) = runs(repeat, || one_layer(&cx, l, lasers))?;
        println!(
            "{}",
            rec(
                "R5_one_layer_everything",
                json!({"layer": l, "at": pos, "laser_rows": laser_rows, "layout": layout}),
                first,
                best,
                sensor_rows
            )
        );
    }

    // R6: a million-sample read of one laser channel from the middle of the build.
    let mid = (layers / 2).max(1);
    let (first, best, rows) = runs(repeat, || {
        let ds = cx.file.dataset(&format!(
            "Build/Lasers/Laser_1/Monitoring_Data/Layer_{mid}/Actual_X"
        ))?;
        Ok(ds.read_raw::<i32>()?.len() as u64)
    })?;
    println!(
        "{}",
        rec(
            "R6_laser_channel_one_layer",
            json!({"layer": mid}),
            first,
            best,
            rows
        )
    );

    // R7: a full scan of one laser channel across the build.
    if a.flag("scan") {
        let t = Instant::now();
        let mut bytes = 0u64;
        for l in 1..=layers {
            let ds = cx.file.dataset(&format!(
                "Build/Lasers/Laser_1/Monitoring_Data/Layer_{l}/Actual_X"
            ))?;
            bytes += ds.read_raw::<i32>()?.len() as u64 * 4;
        }
        let s = t.elapsed().as_secs_f64();
        println!(
            "{}",
            json!({"what": "R7_scan_one_channel_whole_build", "layers": layers, "bytes": bytes, "seconds": s, "mb_s": bytes as f64 / 1e6 / s})
        );
    }
    let _ = LASER_CHANNELS;
    Ok(())
}

fn span(cx: &Ctx) -> Res<(u64, u64)> {
    if cx.build_wide {
        let (mut lo, mut hi) = (u64::MAX, 0u64);
        for s in &cx.sensors {
            let ds = cx
                .file
                .dataset(&format!("Build/Sensors/{s}/Monitoring_Data/Source_Ts"))?;
            let n = ds.size();
            if n == 0 {
                continue;
            }
            lo = lo.min(ds.read_slice_1d::<u64, _>(0..1)?[0]);
            hi = hi.max(ds.read_slice_1d::<u64, _>(n - 1..n)?[0]);
        }
        Ok((lo, hi))
    } else {
        let g = cx.file.group("Build/Layer_Time_Index")?;
        let mn = g.dataset("Min_Ts")?.read_raw::<u64>()?;
        let mx = g.dataset("Max_Ts")?.read_raw::<u64>()?;
        Ok((*mn.iter().min().unwrap(), *mx.iter().max().unwrap()))
    }
}

/// Records with a source time in `lo..=hi`, across every sensor. Returns the number of rows read.
fn window(cx: &Ctx, lo: u64, hi: u64) -> Res<u64> {
    let mut rows = 0u64;
    if cx.build_wide {
        for s in &cx.sensors {
            let g = cx
                .file
                .group(&format!("Build/Sensors/{s}/Monitoring_Data"))?;
            let ts = g.dataset("Source_Ts")?;
            let n = ts.size();
            let idx = g.dataset("Ts_Index")?.read_raw::<u64>()?;
            let c0 = idx.partition_point(|&t| t < lo).saturating_sub(1);
            let c1 = idx.partition_point(|&t| t <= hi);
            let (a, b) = (c0 * cx.chunk, (c1 * cx.chunk).min(n));
            if a >= b {
                continue;
            }
            let win = ts.read_slice_1d::<u64, _>(a..b)?;
            let i0 = a + win.iter().take_while(|&&t| t < lo).count();
            let i1 = a + win.iter().take_while(|&&t| t <= hi).count();
            if i1 > i0 {
                rows += g.dataset("Value")?.read_slice_1d::<f64, _>(i0..i1)?.len() as u64;
            }
        }
    } else {
        let g = cx.file.group("Build/Layer_Time_Index")?;
        let layer = g.dataset("Layer")?.read_raw::<u64>()?;
        let mn = g.dataset("Min_Ts")?.read_raw::<u64>()?;
        let mx = g.dataset("Max_Ts")?.read_raw::<u64>()?;
        for k in 0..layer.len() {
            if mx[k] < lo || mn[k] > hi {
                continue;
            }
            for s in &cx.sensors {
                let Ok(g) = cx.file.group(&format!(
                    "Build/Sensors/{s}/Monitoring_Data/Layer_{}",
                    layer[k]
                )) else {
                    continue;
                };
                let ts = g.dataset("Source_Ts")?.read_raw::<u64>()?;
                let i0 = ts.partition_point(|&t| t < lo);
                let i1 = ts.partition_point(|&t| t <= hi);
                if i1 > i0 {
                    rows += g.dataset("Value")?.read_slice_1d::<f64, _>(i0..i1)?.len() as u64;
                }
            }
        }
    }
    Ok(rows)
}

fn one_sensor(cx: &Ctx, i: usize) -> Res<u64> {
    let s = &cx.sensors[i];
    let mut rows = 0u64;
    if cx.build_wide {
        let g = cx
            .file
            .group(&format!("Build/Sensors/{s}/Monitoring_Data"))?;
        rows += g.dataset("Source_Ts")?.read_raw::<u64>()?.len() as u64;
        rows += g.dataset("Value")?.read_raw::<f64>()?.len() as u64;
    } else {
        for l in 1..=cx.layers {
            let Ok(g) = cx
                .file
                .group(&format!("Build/Sensors/{s}/Monitoring_Data/Layer_{l}"))
            else {
                continue;
            };
            rows += g.dataset("Source_Ts")?.read_raw::<u64>()?.len() as u64;
            rows += g.dataset("Value")?.read_raw::<f64>()?.len() as u64;
        }
    }
    Ok(rows)
}

/// Everything about one layer: all channels of all lasers, and every sensor's records for it.
fn one_layer(cx: &Ctx, layer: u32, lasers: u32) -> Res<(u64, u64)> {
    let mut laser_rows = 0u64;
    for l in 1..=lasers {
        let g = cx.file.group(&format!(
            "Build/Lasers/Laser_{l}/Monitoring_Data/Layer_{layer}"
        ))?;
        for name in &LASER_CHANNELS[..6] {
            laser_rows += g.dataset(name)?.read_raw::<i32>()?.len() as u64;
        }
        laser_rows += g.dataset("Power")?.read_raw::<f32>()?.len() as u64;
        laser_rows += g.dataset("State")?.read_raw::<u8>()?.len() as u64;
    }
    let mut rows = 0u64;
    for s in &cx.sensors {
        if cx.build_wide {
            let li = cx.file.group(&format!("Build/Sensors/{s}/Layer_Index"))?;
            let layers = li.dataset("Layer")?.read_raw::<u32>()?;
            let Some(k) = layers.iter().position(|&x| x == layer) else {
                continue;
            };
            let first = li.dataset("First_Row")?.read_raw::<u64>()?[k] as usize;
            let last = li.dataset("Last_Row")?.read_raw::<u64>()?[k] as usize;
            let g = cx
                .file
                .group(&format!("Build/Sensors/{s}/Monitoring_Data"))?;
            let lab = g.dataset("Layer")?.read_slice_1d::<u32, _>(first..=last)?;
            let n = lab.iter().filter(|&&x| x == layer).count();
            let _ = g
                .dataset("Source_Ts")?
                .read_slice_1d::<u64, _>(first..=last)?;
            let _ = g.dataset("Value")?.read_slice_1d::<f64, _>(first..=last)?;
            rows += n as u64;
        } else {
            let Ok(g) = cx
                .file
                .group(&format!("Build/Sensors/{s}/Monitoring_Data/Layer_{layer}"))
            else {
                continue;
            };
            rows += g.dataset("Source_Ts")?.read_raw::<u64>()?.len() as u64;
            let _ = g.dataset("Value")?.read_raw::<f64>()?;
        }
    }
    Ok((laser_rows, rows))
}
