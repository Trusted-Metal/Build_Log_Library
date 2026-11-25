//! Real laser layers from a V17 build, to tile into a benchmark build (plan step 16). The synthetic generator's power channel
//! and step statistics are not real, and real layers are shorter and vary in length, so size and speed measured on real layers
//! are the honest ones. Only laser data is taken; OPC UA stays synthetic.

use hdf5::File;

use crate::gen::{LaserLayer, LASER_CHANNELS};
use crate::layout::Res;

/// Loads about `want` layers spread across the file, each as `lasers` lasers cut to a common length. Lasers beyond the file's
/// own reuse a neighbouring layer's data. A layer with an empty dataset, or shorter than `min_len`, is skipped.
pub fn load(path: &str, lasers: u32, want: usize, min_len: usize) -> Res<Vec<Vec<LaserLayer>>> {
    let f = File::open(path)?;
    let md = f.group("Built_Part/Monitoring_Data")?;
    let mut ids: Vec<u32> = md
        .member_names()?
        .iter()
        .filter_map(|s| s.strip_prefix("Layer_")?.parse().ok())
        .collect();
    ids.sort_unstable();
    let src_lasers = md
        .group(&format!("Layer_{}", ids[0]))?
        .member_names()?
        .len() as u32;
    let step = (ids.len() / want.max(1)).max(1);
    let read = |layer: u32, laser: u32| -> Res<Option<LaserLayer>> {
        let base = format!("Built_Part/Monitoring_Data/Layer_{layer}/Laser_{laser}");
        let mut pos: Vec<Vec<i32>> = Vec::new();
        for name in &LASER_CHANNELS[..6] {
            pos.push(
                f.dataset(&format!("{base}/Position/{name}"))?
                    .read_raw::<i32>()?,
            );
        }
        let power = f
            .dataset(&format!("{base}/Laser_Data/Laser_Power"))?
            .read_raw::<f32>()?;
        let state = f
            .dataset(&format!("{base}/Laser_Data/Laser_State"))?
            .read_raw::<u8>()?;
        let n = pos
            .iter()
            .map(|v| v.len())
            .chain([power.len(), state.len()])
            .min()
            .unwrap();
        if n < min_len {
            return Ok(None);
        }
        let mut it = pos.into_iter();
        let mut take = || {
            let mut v = it.next().unwrap();
            v.truncate(n);
            v
        };
        let p = [take(), take(), take(), take(), take(), take()];
        let mut power = power;
        power.truncate(n);
        let mut state = state;
        state.truncate(n);
        Ok(Some(LaserLayer {
            pos: p,
            power,
            state,
        }))
    };
    let mut out = Vec::new();
    for (k, _) in ids.iter().enumerate().step_by(step) {
        let mut set = Vec::new();
        for l in 1..=lasers {
            // Often only one laser fires in a layer (in the 2 GB sample, only Laser_1 ever does), so take whichever source laser has
            // data, looking ahead a layer at a time. Each benchmark laser starts from a different layer so the lasers differ.
            let from = k + (l - 1) as usize;
            let mut found = None;
            'search: for off in 0..60 {
                for src_laser in 1..=src_lasers {
                    if let Some(d) = read(ids[(from + off) % ids.len()], src_laser)? {
                        found = Some(d);
                        break 'search;
                    }
                }
            }
            match found {
                Some(d) => set.push(d),
                None => break,
            }
        }
        if set.len() as u32 != lasers {
            continue;
        }
        // one common length for the layer, so that all lasers advance together
        let n = set.iter().map(|d| d.power.len()).min().unwrap();
        for d in &mut set {
            for v in &mut d.pos {
                v.truncate(n);
            }
            d.power.truncate(n);
            d.state.truncate(n);
        }
        out.push(set);
    }
    if out.is_empty() {
        return Err("no usable layers in the file".into());
    }
    Ok(out)
}
