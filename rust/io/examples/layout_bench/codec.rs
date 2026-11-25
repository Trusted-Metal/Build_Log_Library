//! The coordinate encoding of the H5PathOptimizer format ("delta_zigzag"), written from its description: each value becomes its
//! change from the previous one (the first value of every chunk is kept as it is, so a chunk decodes on its own), and each
//! change is zigzagged into a `u32`. Shuffle and deflate are HDF5's own filters and are set on the dataset.
//!
//! The decoded type is `i32`; the stored type is `u32`. Lossless: a delta that does not fit 32 bits is an error, never wrapped.

use hdf5::types::FixedAscii;
use hdf5::Dataset;

use crate::layout::Res;
use crate::Args;

pub const ENCODING_NAME: &str = "delta_zigzag";
pub const ATTR_ENCODING: &str = "h5path_encoding";
pub const ATTR_RESTART: &str = "h5path_delta_restart_points";

#[inline(always)]
fn zigzag(n: i32) -> u32 {
    let u = n as u32;
    (u << 1) ^ 0u32.wrapping_sub(u >> 31)
}

#[inline(always)]
fn unzigzag(z: u32) -> i32 {
    ((z >> 1) ^ 0u32.wrapping_sub(z & 1)) as i32
}

/// Encodes one chunk (the first value restarts the delta) and appends it to `out`.
pub fn encode_chunk(values: &[i32], out: &mut Vec<u32>) -> Result<(), String> {
    let mut prev = 0i32;
    for (i, &v) in values.iter().enumerate() {
        let d = if i == 0 {
            v
        } else {
            let wide = v as i64 - prev as i64;
            i32::try_from(wide).map_err(|_| format!("a delta of {wide} does not fit 32 bits"))?
        };
        out.push(zigzag(d));
        prev = v;
    }
    Ok(())
}

/// Encodes `values` in chunks of `restart`.
pub fn encode(values: &[i32], restart: usize, out: &mut Vec<u32>) -> Result<(), String> {
    out.reserve(values.len());
    for c in values.chunks(restart) {
        encode_chunk(c, out)?;
    }
    Ok(())
}

/// Decodes `stored`, which must start at a restart point, in chunks of `restart`, and appends the values to `out`.
pub fn decode(stored: &[u32], restart: usize, out: &mut Vec<i32>) {
    out.reserve(stored.len());
    for c in stored.chunks(restart) {
        let mut acc = 0i32;
        for (i, &z) in c.iter().enumerate() {
            let d = unzigzag(z);
            acc = if i == 0 { d } else { acc.wrapping_add(d) };
            out.push(acc);
        }
    }
}

pub fn mark_encoded(ds: &Dataset, restart: usize) -> Res<()> {
    let name: FixedAscii<16> = FixedAscii::from_ascii(ENCODING_NAME)?;
    ds.new_attr::<FixedAscii<16>>()
        .create(ATTR_ENCODING)?
        .write_scalar(&name)?;
    ds.new_attr::<u64>()
        .create(ATTR_RESTART)?
        .write_scalar(&(restart as u64))?;
    Ok(())
}

/// The restart interval of an encoded dataset, or `None` if it is stored as it is. An encoding this code does not know is an
/// error and never read as plain values.
pub fn restart_of(ds: &Dataset) -> Res<Option<usize>> {
    let Ok(attr) = ds.attr(ATTR_ENCODING) else {
        return Ok(None);
    };
    let name = attr.read_scalar::<FixedAscii<16>>()?;
    if name.as_str() != ENCODING_NAME {
        return Err(format!("unknown encoding '{}'", name.as_str()).into());
    }
    Ok(Some(ds.attr(ATTR_RESTART)?.read_scalar::<u64>()? as usize))
}

/// All values of a coordinate dataset, decoded if it is encoded. Used by the verifier and the read queries.
pub fn read_i32(ds: &Dataset) -> Res<Vec<i32>> {
    match restart_of(ds)? {
        None => Ok(ds.read_raw::<i32>()?),
        Some(r) => {
            let z = ds.read_raw::<u32>()?;
            let mut out = Vec::with_capacity(z.len());
            decode(&z, r, &mut out);
            Ok(out)
        }
    }
}

/// `codec-bench --path FILE [--layer 1] [--laser 1] [--channel Commanded_X] [--chunk 10000]`: the cost of the translation in Rust,
/// on real coordinates (a V17 file such as `Baseline.h5`): encode and decode speed in memory, and ranged reads through the
/// HDF5 binding, plain against encoded-and-decoded, at distinct offsets so that no read is served from a cache.
pub fn bench(a: &Args) -> Res<()> {
    use hdf5::File;
    use serde_json::json;
    use std::time::Instant;

    let path = a.text("path").ok_or("--path is required")?;
    let layer: u32 = a.get("layer", 1);
    let laser: u32 = a.get("laser", 1);
    let channel = a.text("channel").unwrap_or_else(|| "Commanded_X".into());
    let chunk: usize = a.get("chunk", 10_000);
    let src = File::open(&path)?;
    let values = src
        .dataset(&format!(
            "Built_Part/Monitoring_Data/Layer_{layer}/Laser_{laser}/Position/{channel}"
        ))?
        .read_raw::<i32>()?;
    let n = values.len();
    let mb = n as f64 * 4.0 / 1e6;
    let best = |reps: usize, mut f: Box<dyn FnMut()>| -> f64 {
        (0..reps)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed().as_secs_f64()
            })
            .fold(f64::MAX, f64::min)
    };

    // In memory: the transform alone.
    let mut enc = Vec::new();
    encode(&values, chunk, &mut enc)?;
    let t_enc = best(
        5,
        Box::new({
            let v = values.clone();
            move || {
                let mut o = Vec::new();
                encode(&v, chunk, &mut o).unwrap();
                std::hint::black_box(&o);
            }
        }),
    );
    let t_dec = best(
        5,
        Box::new({
            let e = enc.clone();
            move || {
                let mut o = Vec::new();
                decode(&e, chunk, &mut o);
                std::hint::black_box(&o);
            }
        }),
    );
    let mut round = Vec::new();
    decode(&enc, chunk, &mut round);
    let ok = round == values;
    println!(
        "{}",
        json!({"what": "codec_in_memory", "samples": n, "mb_i32": mb, "roundtrip_ok": ok,
               "encode_mb_s": mb / t_enc, "decode_mb_s": mb / t_dec})
    );

    // Through HDF5: plain and encoded datasets with the same filters (shuffle, deflate 3, fletcher32).
    let dir = std::env::temp_dir().join(format!("codec_bench_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let p = dir.join("c.h5");
    {
        let f = File::create(&p)?;
        let plain = f
            .new_dataset::<i32>()
            .chunk(chunk)
            .shape(n)
            .shuffle()
            .deflate(3)
            .fletcher32()
            .create("plain")?;
        plain.write(&values)?;
        let enc_ds = f
            .new_dataset::<u32>()
            .chunk(chunk)
            .shape(n)
            .shuffle()
            .deflate(3)
            .fletcher32()
            .create("enc")?;
        enc_ds.write(&enc)?;
        mark_encoded(&enc_ds, chunk)?;
    }
    let size_plain;
    let size_enc;
    {
        let f = File::open(&p)?;
        size_plain = f.dataset("plain")?.storage_size();
        size_enc = f.dataset("enc")?.storage_size();
    }
    println!(
        "{}",
        json!({"what": "codec_stored_size", "plain_bytes": size_plain, "encoded_bytes": size_enc, "ratio": size_plain as f64 / size_enc as f64})
    );
    let f = File::open(&p)?;
    let (plain, encd) = (f.dataset("plain")?, f.dataset("enc")?);
    for (_label, len) in [
        ("1000", 1_000usize),
        ("100000", 100_000),
        ("1000000", 1_000_000),
    ] {
        if len > n / 2 {
            continue;
        }
        let reps = 40usize;
        let mut offsets: Vec<usize> = (0..reps)
            .map(|i| 12_345 + i * ((n - len - 20_000) / reps))
            .collect();
        offsets.iter_mut().for_each(|o| *o = (*o).min(n - len - 1));
        let time_reads = |f: &dyn Fn(usize)| -> f64 {
            let t = Instant::now();
            for &o in &offsets {
                f(o);
            }
            t.elapsed().as_secs_f64() * 1000.0 / reps as f64
        };
        let tp = time_reads(&|o| {
            std::hint::black_box(plain.read_slice_1d::<i32, _>(o..o + len).unwrap());
        });
        let te = time_reads(&|o| {
            let s = o - o % chunk;
            let e = ((o + len).div_ceil(chunk) * chunk).min(n);
            let z = encd.read_slice_1d::<u32, _>(s..e).unwrap();
            let mut out = Vec::new();
            decode(z.as_slice().unwrap(), chunk, &mut out);
            std::hint::black_box(&out[o - s..o - s + len]);
        });
        // the same decode but only the HDF5 read of the encoded bytes, to split the cost
        let th = time_reads(&|o| {
            let s = o - o % chunk;
            let e = ((o + len).div_ceil(chunk) * chunk).min(n);
            std::hint::black_box(encd.read_slice_1d::<u32, _>(s..e).unwrap());
        });
        println!(
            "{}",
            json!({"what": "codec_ranged_read_ms", "samples": len, "plain_ms": tp, "encoded_total_ms": te, "encoded_hdf5_read_ms": th,
                   "encoded_decode_ms": te - th, "overhead_x": te / tp})
        );
    }
    let ts = Instant::now();
    let whole = encd.read_raw::<u32>()?;
    let mut out = Vec::new();
    decode(&whole, chunk, &mut out);
    let t_whole = ts.elapsed().as_secs_f64();
    let tp = Instant::now();
    let _ = plain.read_raw::<i32>()?;
    let t_plain = tp.elapsed().as_secs_f64();
    println!(
        "{}",
        json!({"what": "codec_whole_dataset", "plain_ms": t_plain * 1000.0, "encoded_ms": t_whole * 1000.0, "overhead_x": t_whole / t_plain})
    );
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_of_the_format_description_encodes_as_described() {
        // values 20182 20185 20179 20180 20180 -> deltas 20182 +3 -6 +1 0 -> zigzag 40364 6 11 2 0
        let mut out = Vec::new();
        encode(&[20182, 20185, 20179, 20180, 20180], 50_000, &mut out).unwrap();
        assert_eq!(out, vec![40364, 6, 11, 2, 0]);
    }

    #[test]
    fn extremes_and_chunk_restarts_round_trip() {
        let v: Vec<i32> = (0..250_000i64)
            .map(|i| ((i * 7919) % 1_048_576 - 524_288) as i32)
            .chain([i32::MIN / 2, i32::MAX / 2, 0, -1, 1])
            .collect();
        for restart in [1usize, 7, 10_000, 50_000] {
            let mut z = Vec::new();
            encode(&v, restart, &mut z).unwrap();
            let mut back = Vec::new();
            decode(&z, restart, &mut back);
            assert_eq!(back, v, "restart {restart}");
        }
        assert!(encode(&[i32::MIN, i32::MAX], 10, &mut Vec::new()).is_err());
    }
}
