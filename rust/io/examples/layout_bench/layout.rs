//! The two layouts under test. The laser data is written the same way in both (laser-major, one group per laser per layer,
//! as in the V18 fixture); only the **sensor** data differs, which is what F29 is about.
//!
//! * `per_layer`: the fixture's shape. One small dataset pair per sensor per layer. Fallback, with two additions that make
//!   time queries workable: typed values, and `Min_Ts`/`Max_Ts` per dataset plus a build-level `Layer_Time_Index`.
//! * `build_wide`: F29. One chunked, resizable dataset set per sensor for the whole build, sorted by time at `finish`, with a
//!   chunk index (`Ts_Index`), a per-row `Layer` label, and a `Layer_Index` derived at `finish`.

use std::time::Instant;

use hdf5::types::VarLenUnicode;
use hdf5::{Dataset, File, Group, H5Type};

use crate::codec;
use crate::gen::{sensor_names, LaserLayer, OpcRec, LASER_CHANNELS, LASER_EPOCH_US};

pub type Res<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug)]
pub struct Filters {
    pub gzip: u8,
    pub shuffle: bool,
    pub fletcher32: bool,
}

/// The per-dataset chunk cache in bytes for the datasets this harness creates; negative leaves libhdf5's default (1 MiB).
pub static CHUNK_CACHE_BYTES: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

impl Filters {
    pub fn parse(s: &str) -> Res<Filters> {
        Ok(match s {
            "gzip6" => Filters {
                gzip: 6,
                shuffle: false,
                fletcher32: false,
            },
            "gzip6f32" => Filters {
                gzip: 6,
                shuffle: false,
                fletcher32: true,
            },
            "gzip1" => Filters {
                gzip: 1,
                shuffle: false,
                fletcher32: false,
            },
            "f32only" => Filters {
                gzip: 0,
                shuffle: false,
                fletcher32: true,
            },
            "none" => Filters {
                gzip: 0,
                shuffle: false,
                fletcher32: false,
            },
            "gzip6shuffle" => Filters {
                gzip: 6,
                shuffle: true,
                fletcher32: false,
            },
            "gzip1shuffle" => Filters {
                gzip: 1,
                shuffle: true,
                fletcher32: false,
            },
            "gzip3shuffle" => Filters {
                gzip: 3,
                shuffle: true,
                fletcher32: false,
            },
            "gzip1shufflef32" => Filters {
                gzip: 1,
                shuffle: true,
                fletcher32: true,
            },
            "gzip3shufflef32" => Filters {
                gzip: 3,
                shuffle: true,
                fletcher32: true,
            },
            "gzip6shufflef32" => Filters {
                gzip: 6,
                shuffle: true,
                fletcher32: true,
            },
            other => return Err(format!("unknown filters '{other}'").into()),
        })
    }
}

pub fn ensure(parent: &Group, name: &str) -> Res<Group> {
    Ok(parent.group(name).or_else(|_| parent.create_group(name))?)
}

pub fn ensure_path(parent: &Group, path: &str) -> Res<Group> {
    let mut g = parent.clone();
    for part in path.split('/') {
        g = ensure(&g, part)?;
    }
    Ok(g)
}

/// A chunked dataset. `resizable` grows from `len`; otherwise it is fixed at `len` (and the chunk is no larger than it).
pub fn make<T: H5Type>(
    g: &Group,
    name: &str,
    chunk: usize,
    f: &Filters,
    len: usize,
    resizable: bool,
) -> Res<Dataset> {
    let chunk = if resizable {
        chunk
    } else {
        chunk.min(len.max(1))
    };
    let b = g.new_dataset::<T>().chunk(chunk);
    let b = if resizable {
        b.shape(len..)
    } else {
        b.shape(len)
    };
    let b = if f.shuffle { b.shuffle() } else { b };
    let b = if f.gzip > 0 { b.deflate(f.gzip) } else { b };
    let b = if f.fletcher32 { b.fletcher32() } else { b };
    // F31: a chunk cache of about two chunks. libhdf5's default 1 MiB holds written chunks uncompressed until the dataset
    // closes and then compresses them all at once, which stalls the writer at the end of every layer.
    let cache = CHUNK_CACHE_BYTES.load(std::sync::atomic::Ordering::Relaxed);
    let bytes = match cache {
        -2 => None,
        -1 => Some(chunk * std::mem::size_of::<T>() * 2),
        n => Some(n as usize),
    };
    let b = if let Some(nbytes) = bytes {
        b.chunk_cache(521, nbytes, 0.75)
    } else {
        b
    };
    Ok(b.create(name)?)
}

pub fn set_str(g: &Group, name: &str, value: &str) -> Res<()> {
    let v: VarLenUnicode = value.parse()?;
    g.new_attr::<VarLenUnicode>()
        .create(name)?
        .write_scalar(&v)?;
    Ok(())
}

fn append<T: H5Type>(ds: &Dataset, at: usize, data: &[T]) -> Res<()> {
    ds.resize(at + data.len())?;
    ds.write_slice(data, at..at + data.len())?;
    Ok(())
}

// ---- laser data, identical in both layouts -------------------------------------------------------------------------------

pub struct Core {
    pub file: File,
    pub lasers: u32,
    pub filters: Filters,
    pub chunk: usize,
    cur: Vec<Vec<Dataset>>,
    /// Per laser: samples written to the datasets, and samples received so far in the layer (F31: writes are whole chunks).
    written: Vec<usize>,
    avail: Vec<usize>,
    pub layer_us: i64,
    pub laser_samples: usize,
    pub rate_hz: usize,
    /// Write the six coordinate channels delta + zigzag encoded as `u32` (F33).
    pub encode: bool,
    /// Time spent encoding, in nanoseconds (it is part of the laser block latency; reported separately).
    pub encode_ns: u64,
}

impl Core {
    pub fn new(
        file: File,
        lasers: u32,
        filters: Filters,
        chunk: usize,
        rate_hz: usize,
    ) -> Res<Core> {
        let build = ensure(&file, "Build")?;
        ensure(&build, "Lasers")?;
        ensure(&build, "Sensors")?;
        build
            .new_attr::<i32>()
            .create("File_Version")?
            .write_scalar(&18)?;
        Ok(Core {
            file,
            lasers,
            filters,
            chunk,
            cur: Vec::new(),
            written: vec![0; lasers as usize],
            avail: vec![0; lasers as usize],
            layer_us: 0,
            laser_samples: 0,
            rate_hz,
            encode: false,
            encode_ns: 0,
        })
    }

    pub fn begin_layer(&mut self, layer: u32) -> Res<()> {
        self.cur.clear();
        self.laser_samples = 0;
        self.written = vec![0; self.lasers as usize];
        self.avail = vec![0; self.lasers as usize];
        // F30 as amended: shuffle only helps the integer coordinates; on real laser power it doubles the size.
        let plain = Filters {
            shuffle: false,
            ..self.filters
        };
        for l in 1..=self.lasers {
            let g = ensure_path(
                &self.file,
                &format!("Build/Lasers/Laser_{l}/Monitoring_Data/Layer_{layer}"),
            )?;
            let mut dss = Vec::with_capacity(8);
            for name in &LASER_CHANNELS[..6] {
                if self.encode {
                    let ds = make::<u32>(&g, name, self.chunk, &self.filters, 0, true)?;
                    codec::mark_encoded(&ds, self.chunk)?;
                    dss.push(ds);
                } else {
                    dss.push(make::<i32>(&g, name, self.chunk, &self.filters, 0, true)?);
                }
            }
            dss.push(make::<f32>(&g, "Power", self.chunk, &plain, 0, true)?);
            dss.push(make::<u8>(&g, "State", self.chunk, &plain, 0, true)?);
            self.cur.push(dss);
        }
        Ok(())
    }

    /// Samples `a..b` of a laser's layer, all eight channels, as they arrive (a feed batch, for example 500 samples). The
    /// datasets are written in **whole chunks** (F31): the samples wait here until a chunk is full. In this harness the
    /// samples are borrowed from the layer's generated data, so "waiting" is only the bookkeeping of what is written; a real
    /// writer copies them into a buffer, and the spool holds them durably meanwhile.
    pub fn append_laser(&mut self, laser: u32, d: &LaserLayer, a: usize, b: usize) -> Res<()> {
        let i = laser as usize - 1;
        debug_assert_eq!(a, self.avail[i], "batches arrive in order");
        self.avail[i] = b;
        let ready = (self.avail[i] - self.written[i]) / self.chunk * self.chunk;
        if ready > 0 {
            let w = self.written[i];
            self.write_range(i, d, w, w + ready)?;
            self.written[i] += ready;
        }
        Ok(())
    }

    fn write_range(&mut self, i: usize, d: &LaserLayer, a: usize, b: usize) -> Res<()> {
        let dss = self.cur[i].clone();
        for (k, ds) in dss[..6].iter().enumerate() {
            if self.encode {
                // Whole chunks start at a restart point; the last partial chunk of a layer is written once, at the end.
                let t = Instant::now();
                let mut z = Vec::with_capacity(b - a);
                codec::encode(&d.pos[k][a..b], self.chunk, &mut z)?;
                self.encode_ns += t.elapsed().as_nanos() as u64;
                append(ds, a, &z)?;
            } else {
                append(ds, a, &d.pos[k][a..b])?;
            }
        }
        append(&dss[6], a, &d.power[a..b])?;
        append(&dss[7], a, &d.state[a..b])?;
        Ok(())
    }

    /// Flushes everything libhdf5 holds to the operating system (`H5Fflush`).
    pub fn flush_file(&self) -> Res<()> {
        self.file.flush()?;
        Ok(())
    }

    /// Ends the layer: writes the partial chunk that remains (a layer that is not a whole number of chunks), then the layer's
    /// times. `data` is the layer's laser data, one entry per laser.
    pub fn end_layer(&mut self, layer: u32, samples: usize, data: &[LaserLayer]) -> Res<()> {
        for i in 0..self.lasers as usize {
            if self.written[i] < self.avail[i] {
                self.write_range(i, &data[i], self.written[i], self.avail[i])?;
                self.written[i] = self.avail[i];
            }
        }
        let start = self.layer_us;
        let dur = (samples as i64 * 1_000_000) / self.rate_hz as i64;
        for l in 1..=self.lasers {
            let g = self.file.group(&format!(
                "Build/Lasers/Laser_{l}/Monitoring_Data/Layer_{layer}"
            ))?;
            g.new_attr::<i64>()
                .create("Start_Laser_Time")?
                .write_scalar(&(LASER_EPOCH_US + start))?;
            g.new_attr::<i64>()
                .create("End_Laser_Time")?
                .write_scalar(&(LASER_EPOCH_US + start + dur))?;
        }
        self.layer_us += dur;
        self.cur.clear();
        Ok(())
    }
}

pub trait Layout {
    fn core(&mut self) -> &mut Core;
    fn append_opcua(&mut self, layer: u32, recs: &[OpcRec]) -> Res<()>;
    fn end_layer(&mut self, layer: u32) -> Res<()>;
    /// Closes the build's sensor data. Returns the time of each stage in seconds, and a few counters.
    fn finish(&mut self, layers: u32) -> Res<Vec<(String, f64)>>;
}

// ---- per_layer ----------------------------------------------------------------------------------------------------------

pub struct PerLayer {
    pub core: Core,
    names: Vec<String>,
    sensor_chunk: usize,
    buf: Vec<Vec<(u64, f64)>>,
    /// (layer, min ts, max ts) over all sensors, for `Layer_Time_Index`.
    layer_time: Vec<(u64, u64, u64)>,
}

impl PerLayer {
    pub fn new(core: Core, sensors: usize, sensor_chunk: usize) -> Res<PerLayer> {
        set_str(&ensure(&core.file, "Build")?, "Sensor_Layout", "per_layer")?;
        Ok(PerLayer {
            core,
            names: sensor_names(sensors),
            sensor_chunk,
            buf: vec![Vec::new(); sensors],
            layer_time: Vec::new(),
        })
    }
}

impl Layout for PerLayer {
    fn core(&mut self) -> &mut Core {
        &mut self.core
    }
    fn append_opcua(&mut self, _layer: u32, recs: &[OpcRec]) -> Res<()> {
        for r in recs {
            self.buf[r.sensor as usize].push((r.ts, r.value));
        }
        Ok(())
    }
    fn end_layer(&mut self, layer: u32) -> Res<()> {
        let (mut lo, mut hi) = (u64::MAX, 0u64);
        for s in 0..self.names.len() {
            if self.buf[s].is_empty() {
                continue;
            }
            let mut rows = std::mem::take(&mut self.buf[s]);
            rows.sort_by_key(|r| r.0);
            let ts: Vec<u64> = rows.iter().map(|r| r.0).collect();
            let val: Vec<f64> = rows.iter().map(|r| r.1).collect();
            let g = ensure_path(
                &self.core.file,
                &format!(
                    "Build/Sensors/{}/Monitoring_Data/Layer_{layer}",
                    self.names[s]
                ),
            )?;
            let f = self.core.filters;
            let dts = make::<u64>(&g, "Source_Ts", self.sensor_chunk, &f, ts.len(), false)?;
            dts.write(&ts)?;
            let dv = make::<f64>(&g, "Value", self.sensor_chunk, &f, val.len(), false)?;
            dv.write(&val)?;
            let (mn, mx) = (ts[0], ts[ts.len() - 1]);
            g.new_attr::<u64>().create("Min_Ts")?.write_scalar(&mn)?;
            g.new_attr::<u64>().create("Max_Ts")?.write_scalar(&mx)?;
            lo = lo.min(mn);
            hi = hi.max(mx);
        }
        if lo != u64::MAX {
            self.layer_time.push((layer as u64, lo, hi));
        }
        Ok(())
    }
    fn finish(&mut self, _layers: u32) -> Res<Vec<(String, f64)>> {
        let t = Instant::now();
        let g = ensure_path(&self.core.file, "Build/Layer_Time_Index")?;
        let f = self.core.filters;
        let n = self.layer_time.len();
        for (name, pick) in [("Layer", 0usize), ("Min_Ts", 1), ("Max_Ts", 2)] {
            let v: Vec<u64> = self
                .layer_time
                .iter()
                .map(|r| [r.0, r.1, r.2][pick])
                .collect();
            make::<u64>(&g, name, 1024, &f, n, false)?.write(&v)?;
        }
        Ok(vec![("layer_time_index".into(), t.elapsed().as_secs_f64())])
    }
}

// ---- build_wide ---------------------------------------------------------------------------------------------------------

struct SensorWide {
    ts: Vec<u64>,
    val: Vec<f64>,
    layer: Vec<u32>,
    ds: Option<[Dataset; 4]>, // Source_Ts, Value, Layer, Ts_Index
    rows: usize,
    max_ts: u64,
    unsorted: bool,
}

pub struct BuildWide {
    pub core: Core,
    names: Vec<String>,
    chunk: usize,
    s: Vec<SensorWide>,
    /// Flush the partial chunk of every sensor every this many layers (0 = only full chunks, and the tails at `finish`).
    pub tail_every: u32,
}

impl BuildWide {
    pub fn new(core: Core, sensors: usize, sensor_chunk: usize) -> Res<BuildWide> {
        set_str(&ensure(&core.file, "Build")?, "Sensor_Layout", "build_wide")?;
        let s = (0..sensors)
            .map(|_| SensorWide {
                ts: Vec::new(),
                val: Vec::new(),
                layer: Vec::new(),
                ds: None,
                rows: 0,
                max_ts: 0,
                unsorted: false,
            })
            .collect();
        Ok(BuildWide {
            core,
            names: sensor_names(sensors),
            chunk: sensor_chunk,
            s,
            tail_every: 0,
        })
    }

    fn datasets(&mut self, i: usize) -> Res<&[Dataset; 4]> {
        if self.s[i].ds.is_none() {
            let g = ensure_path(
                &self.core.file,
                &format!("Build/Sensors/{}/Monitoring_Data", self.names[i]),
            )?;
            let f = self.core.filters;
            let none = Filters {
                gzip: 0,
                shuffle: false,
                fletcher32: false,
            };
            self.s[i].ds = Some([
                make::<u64>(&g, "Source_Ts", self.chunk, &f, 0, true)?,
                make::<f64>(&g, "Value", self.chunk, &f, 0, true)?,
                make::<u32>(&g, "Layer", self.chunk, &f, 0, true)?,
                make::<u64>(&g, "Ts_Index", 1024, &none, 0, true)?,
            ]);
        }
        Ok(self.s[i].ds.as_ref().unwrap())
    }

    /// Writes the first `n` buffered rows of a sensor. Full chunks only, except for the tail at `finish`.
    fn flush(&mut self, i: usize, n: usize) -> Res<()> {
        if n == 0 {
            return Ok(());
        }
        let chunk = self.chunk;
        self.datasets(i)?;
        let s = &mut self.s[i];
        let ds = s.ds.as_ref().unwrap();
        let at = s.rows;
        append(&ds[0], at, &s.ts[..n])?;
        append(&ds[1], at, &s.val[..n])?;
        append(&ds[2], at, &s.layer[..n])?;
        if at % chunk == 0 {
            append(&ds[3], at / chunk, &s.ts[..1])?;
        }
        s.ts.drain(..n);
        s.val.drain(..n);
        s.layer.drain(..n);
        s.rows += n;
        Ok(())
    }
}

impl Layout for BuildWide {
    fn core(&mut self) -> &mut Core {
        &mut self.core
    }
    fn append_opcua(&mut self, layer: u32, recs: &[OpcRec]) -> Res<()> {
        for r in recs {
            let i = r.sensor as usize;
            let s = &mut self.s[i];
            if r.ts < s.max_ts {
                s.unsorted = true;
            }
            s.max_ts = s.max_ts.max(r.ts);
            s.ts.push(r.ts);
            s.val.push(r.value);
            s.layer.push(layer);
            // Keep chunk alignment even after a partial-chunk flush: write up to the next chunk boundary first.
            loop {
                let room = self.chunk - self.s[i].rows % self.chunk;
                if self.s[i].ts.len() < room {
                    break;
                }
                self.flush(i, room)?;
            }
        }
        Ok(())
    }
    fn end_layer(&mut self, layer: u32) -> Res<()> {
        // Optional: write the partial chunk of every sensor every `tail_every` layers, so that a crash loses at most that
        // many layers of OPC UA data instead of everything since the last full chunk.
        if self.tail_every > 0 && layer % self.tail_every == 0 {
            for i in 0..self.s.len() {
                let n = self.s[i].ts.len();
                self.flush(i, n)?;
            }
        }
        Ok(())
    }
    fn finish(&mut self, _layers: u32) -> Res<Vec<(String, f64)>> {
        let t = Instant::now();
        for i in 0..self.s.len() {
            let n = self.s[i].ts.len();
            self.flush(i, n)?;
        }
        let t_flush = t.elapsed().as_secs_f64();

        // Sort the sensors that received records out of order, and rebuild their chunk index.
        let t = Instant::now();
        let (mut sorted_sensors, mut sorted_rows) = (0u64, 0u64);
        let chunk = self.chunk;
        for i in 0..self.s.len() {
            if !self.s[i].unsorted || self.s[i].ds.is_none() {
                continue;
            }
            let ds = self.s[i].ds.as_ref().unwrap();
            let ts = ds[0].read_raw::<u64>()?;
            let val = ds[1].read_raw::<f64>()?;
            let lay = ds[2].read_raw::<u32>()?;
            let mut idx: Vec<usize> = (0..ts.len()).collect();
            idx.sort_by_key(|&k| (ts[k], k));
            let ts2: Vec<u64> = idx.iter().map(|&k| ts[k]).collect();
            let val2: Vec<f64> = idx.iter().map(|&k| val[k]).collect();
            let lay2: Vec<u32> = idx.iter().map(|&k| lay[k]).collect();
            ds[0].write(&ts2)?;
            ds[1].write(&val2)?;
            ds[2].write(&lay2)?;
            let index: Vec<u64> = ts2.iter().step_by(chunk).copied().collect();
            ds[3].resize(index.len())?;
            ds[3].write(&index)?;
            sorted_sensors += 1;
            sorted_rows += ts2.len() as u64;
        }
        let t_sort = t.elapsed().as_secs_f64();

        // Layer_Index: for each sensor and layer, the first and last row that carry the layer's label.
        let t = Instant::now();
        for i in 0..self.s.len() {
            let Some(ds) = self.s[i].ds.as_ref() else {
                continue;
            };
            let n = self.s[i].rows;
            let mut first: std::collections::BTreeMap<u32, (u64, u64)> =
                std::collections::BTreeMap::new();
            let step = 1 << 20;
            let mut at = 0;
            while at < n {
                let end = (at + step).min(n);
                let lay = ds[2].read_slice_1d::<u32, _>(at..end)?;
                for (k, l) in lay.iter().enumerate() {
                    let row = (at + k) as u64;
                    let e = first.entry(*l).or_insert((row, row));
                    e.1 = row;
                }
                at = end;
            }
            let g = ensure_path(
                &self.core.file,
                &format!("Build/Sensors/{}/Layer_Index", self.names[i]),
            )?;
            let f = self.core.filters;
            let layers: Vec<u32> = first.keys().copied().collect();
            let firsts: Vec<u64> = first.values().map(|v| v.0).collect();
            let lasts: Vec<u64> = first.values().map(|v| v.1).collect();
            make::<u32>(&g, "Layer", 1024, &f, layers.len(), false)?.write(&layers)?;
            make::<u64>(&g, "First_Row", 1024, &f, firsts.len(), false)?.write(&firsts)?;
            make::<u64>(&g, "Last_Row", 1024, &f, lasts.len(), false)?.write(&lasts)?;
        }
        let t_index = t.elapsed().as_secs_f64();
        Ok(vec![
            ("flush_tails".into(), t_flush),
            ("sort_out_of_order".into(), t_sort),
            ("layer_index".into(), t_index),
            ("sensors_sorted".into(), sorted_sensors as f64),
            ("rows_sorted".into(), sorted_rows as f64),
        ])
    }
}
