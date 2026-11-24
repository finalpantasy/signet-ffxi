//! Zone environment as the D3D12 path uses it: one lighting sample turned into shader inputs.
//!
//! Source data and field meanings: `signet_dat::environment`. Without an environment (the
//! zone has no keys, or `--env off`) the renderer keeps the old fixed look: flat blue clear
//! color, no fog, and the neutral hemisphere shading.
use signet_dat::environment::{Environment, Sample};

/// Hour used when no `--hour` is given: noon, so the clear-weather key at `1200` is used as is.
pub const DEFAULT_HOUR: f32 = 12.0;

#[derive(Clone, Copy, Debug)]
pub struct EnvParams {
    pub fog_color: [f32; 3],
    pub fog_start: f32,
    /// Distance where fog is total. 0 or less turns fog off.
    pub fog_end: f32,
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    /// Sky colors at 0, 1/3, 2/3 and 1 of the way from horizon to zenith.
    pub sky: [[f32; 3]; 4],
    /// The key is an indoor/dungeon key (key flag byte 1).
    pub indoor: bool,
    /// Gain on `ambient + diffuse * |N.L|` (tuned, see `OUTDOOR_GAIN` and `INDOOR_GAIN`).
    pub gain: f32,
}

/// Outdoor gain: ambient and diffuse are about 0.7 and 0.85 at noon, so a gain of 0.62 keeps a lit
/// face near the old neutral shading. A tuning guess.
pub const OUTDOOR_GAIN: f32 = 0.62;
/// Indoor/dungeon gain. Dungeon keys carry low light (ambient about 0.31 to 0.37, diffuse about
/// 0.3) and black fog, which at the outdoor gain leaves walls at about a quarter of their texture
/// brightness. 1.0 keeps them dim but readable next to a retail Gusgen Mines screenshot (green-gray
/// stone, bright floor near the camera, fading to black by the fog end). A tuning guess.
pub const INDOOR_GAIN: f32 = 1.0;

impl EnvParams {
    pub fn from_sample(s: &Sample) -> Self {
        // Second diffuse (moon) only counts for ordinary keys; for indoor/event keys that slot
        // holds something that is not a color.
        let extra = if s.flag == 0 { s.second_diffuse } else { [0.0; 3] };
        let diffuse = std::array::from_fn(|k| (s.diffuse[k] + extra[k]).min(1.0));
        // Stops end at 1.0 (zenith); sample the gradient by position along 0..1.
        let sky = std::array::from_fn(|i| s.sky_at(i as f32 / 3.0));
        let indoor = s.flag != 0;
        let gain = std::env::var("SIGNET_GAIN").ok().and_then(|v| v.parse().ok()).unwrap_or(if indoor { INDOOR_GAIN } else { OUTDOOR_GAIN });
        EnvParams { fog_color: s.fog_color, fog_start: s.fog_start, fog_end: s.fog_end, ambient: s.ambient, diffuse, sky, indoor, gain }
    }

    /// Clear color: the horizon sky color.
    pub fn clear(&self) -> [f32; 4] {
        [self.sky[0][0], self.sky[0][1], self.sky[0][2], 1.0]
    }
}

/// Picks weather and hour and returns the shader parameters, or `None` for the old fixed look.
pub fn pick(env: Option<&Environment>, weather: Option<&str>, hour: f32) -> Result<Option<(String, EnvParams)>, String> {
    let Some(env) = env else { return Ok(None) };
    let w = match weather {
        Some(name) => env.weather(name).ok_or_else(|| {
            format!("no weather {name} in this zone (has: {})", env.weathers.iter().map(|w| w.name.as_str()).collect::<Vec<_>>().join(", "))
        })?,
        None => env.default_weather().ok_or("zone environment has no weather")?,
    };
    Ok(w.sample(hour).map(|s| (w.name.clone(), EnvParams::from_sample(&s))))
}
