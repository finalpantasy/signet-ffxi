//! Backend seam: everything the Direct3D 12 and Vulkan renderers share. A backend only turns
//! this data into API calls; pass order, draw sorting, mip chains and frame constants live here.
use crate::actor::{ActorModel, Bone3x4};
use crate::mips;
use crate::scene::{Draw, Pass, Scene};

pub type Res<T> = Result<T, String>;

pub const SKY: [f32; 4] = [0.45, 0.60, 0.80, 1.0];
/// Bone palette capacity: 256 bones of 3 float4 rows each.
pub const PALETTE_BYTES: u64 = 256 * 48;
/// Most copies of the actor one frame draws.
pub const MAX_ACTOR_INSTANCES: usize = 64;

/// Per-frame inputs for a screenshot render.
#[derive(Clone, Copy)]
pub struct FrameParams {
    /// Row-major view-projection.
    pub view_proj: [f32; 16],
    pub light: [f32; 3],
    pub eye: [f32; 3],
}

/// The renderer interface used by the screenshot path.
pub trait Backend {
    fn adapter_name(&self) -> &str;
    /// Uploads a skinned actor. `tex_slots[i]` is the texture slot of `model.images[i]`.
    fn set_actor(&mut self, model: &ActorModel, tex_slots: &[usize]) -> Res<()>;
    /// Writes the bone palette used by the next frame (the previous frame has finished).
    fn set_palette(&mut self, pal: &[Bone3x4]);
    /// Creates the offscreen RGBA8 color target (and depth) for `w` x `h` frames.
    fn begin_offscreen(&mut self, w: u32, h: u32) -> Res<()>;
    /// Draws one frame into the offscreen target and waits for the GPU.
    fn render_offscreen(&mut self, f: &FrameParams) -> Res<()>;
    /// Draws one frame and returns it as tightly packed RGBA8, alpha forced to 255.
    fn render_readback(&mut self, f: &FrameParams) -> Res<Vec<u8>>;
}

/// The 20 floats of frame constants: view-proj, then light xyz and the vertex-color flag.
pub fn frame_constants(f: &FrameParams, vertex_colors: bool) -> [f32; 20] {
    let mut c = [0f32; 20];
    c[..16].copy_from_slice(&f.view_proj);
    c[16..19].copy_from_slice(&f.light);
    c[19] = if vertex_colors { 1.0 } else { 0.0 };
    c
}

/// Draws of one pass in submission order; blended draws sort back to front.
pub fn pass_draws(draws: &[Draw], pass: Pass, eye: [f32; 3]) -> Vec<&Draw> {
    let mut v: Vec<&Draw> = draws.iter().filter(|d| d.pass == pass).collect();
    if pass == Pass::Blend {
        let dist2 = |d: &Draw| (0..3).map(|k| (d.center[k] - eye[k]).powi(2)).sum::<f32>();
        v.sort_by(|a, b| dist2(b).total_cmp(&dist2(a)));
    }
    v
}

/// Texture slot 0 is a 1x1 gray fallback; slot n > 0 is `scene.slots[n - 1]`. Each entry is the
/// full mip chain (level 0 first). SIGNET_MIPS=box skips the coverage-preserving chain.
pub fn texture_chains(scene: &Scene) -> Vec<Vec<mips::Level>> {
    let gray = [150u8, 150, 150, 255];
    let all: Vec<(&[u8], u32, u32, mips::TexClass)> = std::iter::once((&gray[..], 1, 1, mips::TexClass::Opaque))
        .chain(scene.slots.iter().map(|&(i, c)| (&scene.images[i].rgba[..], scene.images[i].width, scene.images[i].height, c)))
        .collect();
    let box_mips = std::env::var("SIGNET_MIPS").is_ok_and(|v| v == "box");
    all.iter()
        .map(|(rgba, w, h, class)| {
            let class = if *class == mips::TexClass::Cutout && box_mips { mips::TexClass::Translucent } else { *class };
            mips::chain(rgba, *w, *h, class)
        })
        .collect()
}

/// (texture slot, first index, index count) per actor draw.
pub fn actor_draws(model: &ActorModel, tex_slots: &[usize]) -> Vec<(usize, u32, u32)> {
    model.draws.iter().map(|d| (d.image.map_or(0, |i| tex_slots[i]), d.first_index, d.index_count)).collect()
}

/// Views a slice of plain-old-data as bytes.
pub fn bytes_of<T: Copy>(v: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}
