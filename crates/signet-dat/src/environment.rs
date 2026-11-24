//! Zone environment (weather and time-of-day lighting keys), read from the zone's own DAT.
//!
//! Findings (original work from the install's chunk stream; field meaning is inferred from the
//! values and marked as such):
//! - The chunk stream is a tree: kind `0x01` opens a folder (name = the chunk name) and kind
//!   `0x00` ("end.") closes it. Under the folder `weat` sit one folder per weather (`suny`, `fine`,
//!   `clod`, `mist`, `bliz`, `thdr`, `rain`, `dust`, `wind` ...). Each weather folder holds
//!   kind `0x2f` chunks named by the time of day as `HHMM` (`0000`, `0500`, `0600`, `1200`,
//!   `1700`, `1800`, `2100`; some zones have only `0000` and `1200`). Every key is 176 bytes.
//!   Zones above id 255 use the same layout (zone 256's keys are in file 83891).
//! - Key layout (little endian). Decoded by pattern, not from a spec:
//!   - `0x00` u8 flag: 0 for ordinary weather keys, 1 for indoor/event keys. When 1, color 1 is
//!     not a color (it reads `017f01`).
//!   - Two 32-byte light blocks at `0x0c` and `0x2c`, each four RGBA colors (alpha always 0x80,
//!     the PS2 "opaque") then four f32. The blocks differ only slightly (the second is a little
//!     darker); only the first is used here.
//!     color 0 = sun/diffuse (black at 0000, bright at 1200), color 1 = second diffuse light that
//!     is bright at 0000 and 0500 and dark otherwise (read as moon), color 2 = ambient,
//!     color 3 = fog/background color (in event keys the whole sky collapses to this color).
//!     f32 0 (offset `0x1c`) = fog end distance (mist 69..99, clear 340..450, indoor 64..89, so it
//!     tracks visibility); f32 1 (`0x20`) = fog start (0 outdoors, 2..5 in dungeons); f32 2 =
//!     1.1..1.75, meaning unknown (not used).
//!   - `0x4c` RGBA sky horizon color, then constants (56.4, 0, 252.5 and `0x000a0002`-style words
//!     and 2029.5) whose meaning is unknown (not used).
//!   - `0x6c` eight RGBA sky gradient colors (horizon first, zenith last), then at `0x90` eight
//!     f32 stops (0.165 0.273 0.381 0.49 0.66 0.83 1.0 0). Indoor keys have all-black gradients.
//! - Not decoded: directions of the sun and moon (not in the key), which block applies to actors
//!   versus terrain, the unknown constants above.

use crate::chunk::chunks;

/// Bytes in one key chunk payload.
pub const KEY_LEN: usize = 176;
/// Chunk kind of a time-of-day key.
pub const KEY_KIND: u8 = 0x2f;

/// One time-of-day key (colors are 0..=1 RGB). Also the type of an interpolated sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    /// Minutes since midnight.
    pub minute: u16,
    pub flag: u8,
    pub diffuse: [f32; 3],
    pub second_diffuse: [f32; 3],
    pub ambient: [f32; 3],
    pub fog_color: [f32; 3],
    pub fog_end: f32,
    pub fog_start: f32,
    pub horizon: [f32; 3],
    pub sky: [[f32; 3]; 8],
    pub stops: [f32; 8],
}

/// Interpolated lighting at one time of day.
pub type Sample = Key;

#[derive(Debug, Clone)]
pub struct Weather {
    pub name: String,
    /// Sorted by time of day.
    pub keys: Vec<Key>,
}

fn rgb(b: &[u8], o: usize) -> [f32; 3] {
    [b[o] as f32 / 255.0, b[o + 1] as f32 / 255.0, b[o + 2] as f32 / 255.0]
}
fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

impl Key {
    /// Parses a key payload and its `HHMM` chunk name.
    pub fn parse(name: &[u8; 4], p: &[u8]) -> Option<Key> {
        if p.len() < KEY_LEN || !name.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let d = |i: usize| (name[i] - b'0') as u16;
        let (h, m) = (d(0) * 10 + d(1), d(2) * 10 + d(3));
        if h > 24 || m > 59 {
            return None;
        }
        let mut sky = [[0.0; 3]; 8];
        let mut stops = [0.0; 8];
        for i in 0..8 {
            sky[i] = rgb(p, 0x6c + 4 * i);
            stops[i] = f32_at(p, 0x90 + 4 * i);
        }
        let (fog_end, fog_start) = (f32_at(p, 0x1c), f32_at(p, 0x20));
        if !(fog_end.is_finite() && fog_start.is_finite()) {
            return None;
        }
        Some(Key {
            minute: h * 60 + m,
            flag: p[0],
            diffuse: rgb(p, 0x0c),
            second_diffuse: rgb(p, 0x10),
            ambient: rgb(p, 0x14),
            fog_color: rgb(p, 0x18),
            fog_end,
            fog_start,
            horizon: rgb(p, 0x4c),
            sky,
            stops,
        })
    }

    fn lerp(&self, o: &Key, t: f32, minute: f32) -> Key {
        let l = |a: f32, b: f32| a + (b - a) * t;
        let l3 = |a: [f32; 3], b: [f32; 3]| [l(a[0], b[0]), l(a[1], b[1]), l(a[2], b[2])];
        let mut sky = [[0.0; 3]; 8];
        let mut stops = [0.0; 8];
        for i in 0..8 {
            sky[i] = l3(self.sky[i], o.sky[i]);
            stops[i] = l(self.stops[i], o.stops[i]);
        }
        Key {
            minute: minute as u16,
            flag: self.flag.max(o.flag),
            diffuse: l3(self.diffuse, o.diffuse),
            second_diffuse: l3(self.second_diffuse, o.second_diffuse),
            ambient: l3(self.ambient, o.ambient),
            fog_color: l3(self.fog_color, o.fog_color),
            fog_end: l(self.fog_end, o.fog_end),
            fog_start: l(self.fog_start, o.fog_start),
            horizon: l3(self.horizon, o.horizon),
            sky,
            stops,
        }
    }

    /// Sky color at `t` (0 = horizon, 1 = zenith) along the 7 gradient stops. All-black gradients
    /// (indoor keys) return the fog color instead.
    pub fn sky_at(&self, t: f32) -> [f32; 3] {
        if self.sky[..7].iter().all(|c| c.iter().all(|&v| v < 1.0 / 255.0)) {
            return self.fog_color;
        }
        if t <= self.stops[0] {
            return self.sky[0];
        }
        for i in 1..7 {
            if t <= self.stops[i] {
                let w = ((t - self.stops[i - 1]) / (self.stops[i] - self.stops[i - 1]).max(1e-4)).clamp(0.0, 1.0);
                return std::array::from_fn(|k| self.sky[i - 1][k] + (self.sky[i][k] - self.sky[i - 1][k]) * w);
            }
        }
        self.sky[6]
    }
}

impl Weather {
    /// Lighting at `hour` (0..24, fractional), interpolated between the neighboring keys and
    /// wrapping past midnight. `None` when the weather has no keys.
    pub fn sample(&self, hour: f32) -> Option<Sample> {
        let n = self.keys.len();
        if n == 0 {
            return None;
        }
        let m = hour.rem_euclid(24.0) * 60.0;
        // Last key at or before m, else the last key of the previous day.
        let prev = self.keys.iter().rposition(|k| k.minute as f32 <= m).unwrap_or(n - 1);
        let next = (prev + 1) % n;
        let (a, b) = (&self.keys[prev], &self.keys[next]);
        let am = a.minute as f32;
        let mut bm = b.minute as f32;
        let mut mm = m;
        if bm <= am {
            bm += 1440.0;
        }
        if mm < am {
            mm += 1440.0;
        }
        let t = if bm > am { ((mm - am) / (bm - am)).clamp(0.0, 1.0) } else { 0.0 };
        Some(a.lerp(b, t, m))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub weathers: Vec<Weather>,
}

impl Environment {
    /// Reads every time-of-day key in a zone DAT, grouped by the folder that holds it. Folders
    /// under `weat` are the weather set; if the zone has none, any folder with keys counts.
    pub fn parse(dat: &[u8]) -> Option<Environment> {
        let mut path: Vec<[u8; 4]> = Vec::new();
        let mut under_weat: Vec<Weather> = Vec::new();
        let mut other: Vec<Weather> = Vec::new();
        for c in chunks(dat).filter_map(Result::ok) {
            match c.kind {
                0x00 => {
                    path.pop();
                }
                0x01 => path.push(c.name),
                KEY_KIND => {
                    let Some(key) = Key::parse(&c.name, c.data) else { continue };
                    let Some(parent) = path.last() else { continue };
                    let name: String = parent.iter().map(|&b| b as char).collect();
                    let in_weat = path.len() >= 2 && &path[path.len() - 2] == b"weat";
                    let list = if in_weat { &mut under_weat } else { &mut other };
                    match list.iter_mut().find(|w| w.name == name) {
                        Some(w) => w.keys.push(key),
                        None => list.push(Weather { name, keys: vec![key] }),
                    }
                }
                _ => {}
            }
        }
        let mut weathers = if under_weat.is_empty() { other } else { under_weat };
        for w in &mut weathers {
            w.keys.sort_by_key(|k| k.minute);
            w.keys.dedup_by_key(|k| k.minute);
        }
        (!weathers.is_empty()).then_some(Environment { weathers })
    }

    pub fn weather(&self, name: &str) -> Option<&Weather> {
        self.weathers.iter().find(|w| w.name.eq_ignore_ascii_case(name))
    }

    /// Clear-sky weather if present (`suny`, then `fine`), else the first one in the file.
    pub fn default_weather(&self) -> Option<&Weather> {
        self.weather("suny").or_else(|| self.weather("fine")).or_else(|| self.weathers.first())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_bytes(flag: u8, ambient: [u8; 3], fog_end: f32) -> Vec<u8> {
        let mut p = vec![0u8; KEY_LEN];
        p[0] = flag;
        p[0x14..0x17].copy_from_slice(&ambient);
        p[0x1c..0x20].copy_from_slice(&fog_end.to_le_bytes());
        p
    }

    fn chunk(name: &[u8; 4], kind: u8, payload: &[u8]) -> Vec<u8> {
        let units = (16 + payload.len()).div_ceil(16);
        let mut b = name.to_vec();
        b.extend(((kind as u32) | ((units as u32) << 7)).to_le_bytes());
        b.extend([0u8; 8]);
        b.extend(payload);
        b.resize(units * 16, 0);
        b
    }

    #[test]
    fn parses_weather_tree_and_interpolates() {
        let mut dat = chunk(b"zone", 1, &[]);
        dat.extend(chunk(b"weat", 1, &[]));
        dat.extend(chunk(b"fine", 1, &[]));
        dat.extend(chunk(b"1200", KEY_KIND, &key_bytes(0, [200, 200, 200], 400.0)));
        dat.extend(chunk(b"0000", KEY_KIND, &key_bytes(0, [0, 0, 100], 200.0)));
        dat.extend(chunk(b"end.", 0, &[]));
        dat.extend(chunk(b"end.", 0, &[]));
        dat.extend(chunk(b"ev01", 1, &[])); // ignored: not under weat
        dat.extend(chunk(b"1200", KEY_KIND, &key_bytes(1, [9, 9, 9], 5.0)));
        dat.extend(chunk(b"end.", 0, &[]));
        dat.extend(chunk(b"end.", 0, &[]));
        let env = Environment::parse(&dat).unwrap();
        assert_eq!(env.weathers.len(), 1);
        let w = env.default_weather().unwrap();
        assert_eq!(w.name, "fine");
        assert_eq!(w.keys.iter().map(|k| k.minute).collect::<Vec<_>>(), [0, 720]);
        let noon = w.sample(12.0).unwrap();
        assert_eq!((noon.fog_end, noon.ambient[0]), (400.0, 200.0 / 255.0));
        let mid = w.sample(6.0).unwrap();
        assert!((mid.fog_end - 300.0).abs() < 1e-3, "{}", mid.fog_end);
        // After the last key it blends back toward the first key of the next day.
        let late = w.sample(18.0).unwrap();
        assert!((late.fog_end - 300.0).abs() < 1e-3, "{}", late.fog_end);
    }

    #[test]
    fn rejects_bad_names_and_short_payloads() {
        assert!(Key::parse(b"abcd", &[0; KEY_LEN]).is_none());
        assert!(Key::parse(b"1200", &[0; 40]).is_none());
        assert!(Key::parse(b"2599", &[0; KEY_LEN]).is_none());
    }

    #[test]
    fn install_zone_has_environment() {
        let Some(root) = crate::find_install_root() else { return };
        let install = crate::Install::open(root).unwrap();
        let Ok(dat) = install.read(crate::zones::model_file_id(100)) else { return };
        let env = Environment::parse(&dat).expect("West Ronfaure has weather keys");
        let s = env.default_weather().unwrap().sample(12.0).unwrap();
        assert!(s.fog_end > 100.0 && s.ambient.iter().all(|&c| c > 0.2));
    }
}
