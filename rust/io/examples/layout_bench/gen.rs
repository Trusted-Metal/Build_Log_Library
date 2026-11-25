//! A synthetic build: scan-vector laser data and OPC UA records. It is **not** real data, and the compression ratios it
//! reaches are tuned (`calibrate`) to the ones measured on the V18 fixture, because compression decides both the file size
//! and the CPU a writer spends. Deterministic for a given seed.

/// xorshift64*: small, fast, deterministic.
pub struct Rng(pub u64);

impl Rng {
    /// A generator for one (layer, stream), so that any layer can be regenerated exactly, in any order. The verifier of the
    /// kill test needs this to know what a crashed file should contain.
    pub fn seeded(base: u64, layer: u32, stream: u32) -> Rng {
        let mut r =
            Rng(base ^ ((layer as u64) << 32) ^ ((stream as u64) << 8) ^ 0x9E37_79B9_7F4A_7C15);
        for _ in 0..4 {
            r.next();
        }
        r
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in `-a..=a`.
    pub fn sym(&mut self, a: i32) -> i32 {
        if a <= 0 {
            return 0;
        }
        (self.next() % (2 * a as u64 + 1)) as i32 - a
    }
}

/// How noisy each channel is, in least-significant bits. Tuned so that gzip 6 reaches the fixture's ratios.
#[derive(Clone, Debug)]
pub struct LaserParams {
    pub cmd_noise: [i32; 3],
    pub act_noise: [i32; 3],
    pub power_noise_lsb: i32,
}

impl Default for LaserParams {
    fn default() -> Self {
        LaserParams {
            cmd_noise: [1, 7, 3],
            act_noise: [60, 60, 20],
            power_noise_lsb: 2,
        }
    }
}

/// Fixture ratios (raw bytes over stored bytes, chunk 10000, gzip 6) measured on `V18_fixture.h5`, in channel order:
/// Actual X, Y, Z, Commanded X, Y, Z, Power, State.
pub const FIXTURE_RATIOS: [f64; 8] = [2.66, 2.66, 3.8, 4.73, 4.64, 6.55, 6.83, 108.0];

pub const LASER_CHANNELS: [&str; 8] = [
    "Actual_X",
    "Actual_Y",
    "Actual_Z",
    "Commanded_X",
    "Commanded_Y",
    "Commanded_Z",
    "Power",
    "State",
];

pub struct LaserLayer {
    /// Actual X, Y, Z then Commanded X, Y, Z.
    pub pos: [Vec<i32>; 6],
    pub power: Vec<f32>,
    pub state: Vec<u8>,
}

const LINE: usize = 8000;
const JUMP: usize = 400;
const DX: i64 = 6;
/// The commanded position is updated every `HOLD` samples (the scan controller's command rate is below the sample rate).
const HOLD: usize = 4;
const PITCH: i64 = 2000;
const LAG: usize = 4;
const DAC_LSB: f64 = 10.0 / 65536.0;

pub fn laser_layer(rng: &mut Rng, p: &LaserParams, laser: u32, layer: u32, n: usize) -> LaserLayer {
    let period = LINE + JUMP;
    let mut cmd: [Vec<i32>; 3] = [
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    ];
    let mut power = Vec::with_capacity(n);
    let mut state = Vec::with_capacity(n);
    let x0 = -(LINE as i64 * DX) / 2 + laser as i64 * 1_000_000;
    let z = layer as i64 * 300;
    let mut level = 5.0f64;
    let mut line_idx = 0usize;
    for i in 0..n {
        let phase = i % period;
        if phase == 0 {
            line_idx = i / period;
            level = 3.0 + 5.0 * rng.unit();
        }
        let dir: i64 = if line_idx % 2 == 0 { 1 } else { -1 };
        let island_line = (line_idx % 400) as i64;
        let (x, y, on, v) = if phase < LINE {
            let along = (phase / HOLD * HOLD) as i64 * DX;
            let x = if dir > 0 {
                x0 + along
            } else {
                x0 + LINE as i64 * DX - along
            };
            let y = island_line * PITCH;
            let ramp = (phase.min(150) as f64 / 150.0).min((LINE - phase).min(150) as f64 / 150.0);
            (x, y, 1u8, level * ramp)
        } else {
            let t = (phase - LINE) as i64;
            let x_end = if dir > 0 { x0 + LINE as i64 * DX } else { x0 };
            let y = island_line * PITCH + PITCH * t / JUMP as i64;
            (x_end, y, 0u8, 0.0)
        };
        cmd[0].push(x as i32 + rng.sym(p.cmd_noise[0]));
        cmd[1].push(y as i32 + rng.sym(p.cmd_noise[1]));
        cmd[2].push(z as i32 + rng.sym(p.cmd_noise[2]));
        let q = (v / DAC_LSB).round() + rng.sym(p.power_noise_lsb) as f64;
        power.push((q.max(0.0) * DAC_LSB) as f32);
        state.push(on);
    }
    let act = |axis: usize, rng: &mut Rng| -> Vec<i32> {
        (0..n)
            .map(|i| cmd[axis][i.saturating_sub(LAG)] + rng.sym(p.act_noise[axis]))
            .collect()
    };
    let (ax, ay, az) = (act(0, rng), act(1, rng), act(2, rng));
    let [cx, cy, cz] = cmd;
    LaserLayer {
        pos: [ax, ay, az, cx, cy, cz],
        power,
        state,
    }
}

// ---- OPC UA ------------------------------------------------------------------------------------------------------------

/// Epoch of the synthetic OPC UA clock, in nanoseconds. It deliberately differs from the laser clock (the FPGA and the
/// PLC do not share a hardware clock).
pub const OPC_EPOCH_NS: u64 = 1_787_072_590_000_000_000;
pub const LASER_EPOCH_US: i64 = 1_787_071_705_620_000;

#[derive(Clone, Copy, Debug)]
pub struct OpcRec {
    pub sensor: u16,
    pub ts: u64,
    pub value: f64,
    /// When the record reached ClearBox, in nanoseconds from the start of the build; records are fed in this order.
    pub arrival_ns: u64,
}

pub fn sensor_names(n: usize) -> Vec<String> {
    const SUBSYSTEMS: [&str; 7] = [
        "Actuators",
        "Chamber",
        "Gas",
        "Optics",
        "Power",
        "Recoater",
        "Thermal",
    ];
    (0..n)
        .map(|i| {
            format!(
                "{}_Node_{:02}_Signal_OPCUA",
                SUBSYSTEMS[i % SUBSYSTEMS.len()],
                i
            )
        })
        .collect()
}

pub struct OpcState {
    pub rate_hz: Vec<f64>,
    pub value: Vec<f64>,
    /// Fraction of records that arrive with a source time up to two seconds earlier than the previous one (out of order).
    pub disorder: f64,
}

impl OpcState {
    pub fn new(sensors: usize, disorder: f64) -> OpcState {
        const RATES: [f64; 7] = [0.1, 0.5, 1.0, 2.0, 2.0, 5.0, 10.0];
        OpcState {
            rate_hz: (0..sensors).map(|i| RATES[i % RATES.len()]).collect(),
            value: (0..sensors).map(|i| 20.0 + i as f64).collect(),
            disorder,
        }
    }
}

/// The records of one layer, sorted by arrival. The values start from a fixed point for the layer (not from the previous
/// layer's state), so a layer can be regenerated on its own.
pub fn opcua_layer(
    rng: &mut Rng,
    st: &mut OpcState,
    layer: u32,
    layer_start_ns: u64,
    layer_ns: u64,
) -> Vec<OpcRec> {
    for (s, v) in st.value.iter_mut().enumerate() {
        *v = 20.0 + s as f64 + layer as f64 * 0.001;
    }
    let mut out = Vec::new();
    for s in 0..st.rate_hz.len() {
        let count =
            ((st.rate_hz[s] * layer_ns as f64 / 1e9) * (0.8 + 0.4 * rng.unit())).round() as u64;
        for k in 0..count {
            let arrival = layer_start_ns + (k * layer_ns) / count.max(1) + (rng.next() % 1_000_000);
            if rng.unit() < 0.3 {
                st.value[s] += (rng.unit() - 0.5) * 0.2;
            }
            let mut ts = OPC_EPOCH_NS + arrival;
            if rng.unit() < st.disorder {
                ts = ts.saturating_sub(rng.next() % 2_000_000_000);
            }
            out.push(OpcRec {
                sensor: s as u16,
                ts,
                value: st.value[s],
                arrival_ns: arrival,
            });
        }
    }
    out.sort_by_key(|r| r.arrival_ns);
    out
}
