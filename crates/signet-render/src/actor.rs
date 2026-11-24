//! Skinned actor loading and CPU-side pose math (animation interpolation, bone palette,
//! reference skinning). No GPU types here so everything is unit-testable.
use std::collections::HashMap;

use signet_dat::skeleton::{Pose, Skeleton, quat_normalize};
use signet_dat::texture::{self, Image};
use signet_dat::{Install, actor_mesh, anim, chunks};

/// DAT ids of the default Hume male: skeleton + animations, then face, body, hands, legs, feet.
pub const HUME_SKELETON: u32 = 7072;
pub const HUME_PARTS: [u32; 5] = [7080, 7368, 7598, 7880, 8110];
/// Animation frames run at 30 per second times the animation's own speed.
pub const FRAME_RATE: f32 = 30.0;

/// GPU vertex: up to two bone influences, positions already weighted (dual-bone) and local to
/// each bone. Mirror twins are baked into separate vertices on the CPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActorVertex {
    pub p0: [f32; 3],
    pub n0: [f32; 3],
    pub p1: [f32; 3],
    pub n1: [f32; 3],
    /// Influence weights; used to scale the bone translation (positions are pre-weighted).
    pub w: [f32; 2],
    pub bone: [u32; 2],
    pub uv: [f32; 2],
}

/// One texture's triangle range in `ActorModel::indices`.
#[derive(Clone, Debug)]
pub struct ActorDraw {
    /// Index into `ActorModel::images`, or None for the gray fallback.
    pub image: Option<usize>,
    pub first_index: u32,
    pub index_count: u32,
}

pub struct ActorModel {
    pub skeleton: Skeleton,
    pub bind_local: Vec<Pose>,
    pub anims: Vec<(String, anim::Animation)>,
    pub vertices: Vec<ActorVertex>,
    pub indices: Vec<u32>,
    pub draws: Vec<ActorDraw>,
    pub images: Vec<texture::Image>,
    /// Attachment bones (81 and up) the meshes are skinned to: the weapon mounts. Their pose
    /// comes from the weapon animation set (`idl1`, `run1`, ...), not from `idl0`/`run0`.
    pub weapon_bones: Vec<u32>,
    /// Draw the weapon in the right hand (bone [`GRIP_RIGHT`]) instead of at its sheath mount.
    pub engaged: bool,
}

/// First weapon-attachment bone. Bones 81..=107 hold no body geometry: weapon meshes skin to them
/// and the weapon animation sets (`wlk1`, `idl1`, `run1` ... with the digit 1) key them to the
/// sheath mounts (decoded 2026-11-23); the digit-0 set leaves them unkeyed at the pelvis.
pub const ATTACH_FIRST: u16 = 81;
/// Right-hand grip bone of the humanoid skeletons (child of the right hand; its -z axis is the
/// blade direction, taken from the reference-point list). Verified on the Mithra skeleton only.
pub const GRIP_RIGHT: usize = 78;

fn bone_index(bone: u16, bones: usize) -> u32 {
    // Out-of-range bones map to the identity entry appended after the real bones.
    if (bone as usize) < bones { bone as u32 } else { bones as u32 }
}

fn make_vertex(v: &actor_mesh::SkinVertex, mirrored: bool, bones: usize) -> ActorVertex {
    let mut out = ActorVertex::default();
    // A one-influence vertex leaves slot 1 empty (zero weight, zero position, identity bone).
    out.bone = [bones as u32; 2];
    for (slot, inf) in v.influences.iter().take(2).enumerate() {
        let (bone, axis) = if mirrored { (inf.bone_mirror, inf.mirror_axis) } else { (inf.bone, 0) };
        let (mut p, mut n) = (inf.position, inf.normal);
        if axis > 0 {
            p[axis as usize - 1] = -p[axis as usize - 1];
            n[axis as usize - 1] = -n[axis as usize - 1];
        }
        out.bone[slot] = bone_index(bone, bones);
        out.w[slot] = inf.weight;
        if slot == 0 {
            (out.p0, out.n0) = (p, n);
        } else {
            (out.p1, out.n1) = (p, n);
        }
    }
    out
}

/// Loads the default Hume from an install. Textures come out decoded with the zone viewer's
/// DXT3 alpha fix applied.
pub fn load_hume(install: &Install) -> Result<ActorModel, String> {
    let ids: Vec<u32> = std::iter::once(HUME_SKELETON).chain(HUME_PARTS).collect();
    load_actor(install, &ids)
}

/// Loads an actor from DAT ids: the first must read and normally holds the skeleton (monsters
/// hold everything in that one DAT); the rest are body parts (faces, armor) and are skipped when
/// they are missing. Meshes, textures and animations are gathered from every DAT.
pub fn load_actor(install: &Install, ids: &[u32]) -> Result<ActorModel, String> {
    let mut images: Vec<Image> = Vec::new();
    let mut tex_by_name: HashMap<String, usize> = HashMap::new();
    let mut skeleton: Option<Skeleton> = None;
    // Each mesh remembers its DAT: a face and a helmet both name their texture "hm_h", so a mesh
    // looks in its own DAT first.
    let mut meshes: Vec<(usize, actor_mesh::ActorMesh)> = Vec::new();
    let mut local_tex: Vec<HashMap<String, usize>> = vec![HashMap::new(); ids.len()];
    let mut anims = Vec::new();
    for (n, &id) in ids.iter().enumerate() {
        let dat = match install.read(id) {
            Ok(d) => d,
            Err(e) if n == 0 => return Err(format!("read DAT {id}: {e}")),
            Err(_) => continue,
        };
        for c in chunks(&dat).filter_map(Result::ok) {
            match c.kind {
                texture::KIND => {
                    if let Ok((h, img)) = texture::decode(c.data) {
                        if img.width == 0 || img.height == 0 || img.rgba.len() < (img.width * img.height * 4) as usize {
                            continue;
                        }
                        let full = format!("{:<8}{:<8}", h.group, h.name).trim_end().to_string();
                        local_tex[n].insert(full.clone(), images.len());
                        local_tex[n].entry(h.name.clone()).or_insert(images.len());
                        tex_by_name.insert(full, images.len());
                        tex_by_name.entry(h.name.clone()).or_insert(images.len());
                        images.push(img);
                    }
                }
                signet_dat::skeleton::KIND if skeleton.is_none() => skeleton = signet_dat::skeleton::parse(c.data).ok(),
                actor_mesh::KIND => {
                    if let Ok(m) = actor_mesh::parse(c.data) {
                        meshes.push((n, m));
                    }
                }
                anim::KIND => {
                    if let Ok(a) = anim::parse(c.data) {
                        anims.push((c.name_str().trim_end_matches('.').to_string(), a));
                    }
                }
                _ => {}
            }
        }
    }
    let skeleton = skeleton.ok_or("actor DATs have no skeleton")?;
    let bones = skeleton.bones.len();
    let mut vertices: Vec<ActorVertex> = Vec::new();
    // Triangles per image so each draw binds one texture.
    let mut by_image: std::collections::BTreeMap<Option<usize>, Vec<u32>> = Default::default();
    for (dat_n, m) in &meshes {
        let twins: &[bool] = if m.flip { &[false, true] } else { &[false] };
        for &mirrored in twins {
            let base: Vec<ActorVertex> = m.vertices.iter().map(|v| make_vertex(v, mirrored, bones)).collect();
            for g in &m.groups {
                let image = local_tex[*dat_n].get(&g.texture).or_else(|| tex_by_name.get(&g.texture)).copied();
                let list = by_image.entry(image).or_default();
                for t in g.corners.chunks_exact(3) {
                    for c in t {
                        // Corner UVs are per triangle corner, so each corner becomes its own vertex.
                        let mut v = base[c.vertex as usize];
                        v.uv = c.uv;
                        list.push(vertices.len() as u32);
                        vertices.push(v);
                    }
                }
            }
        }
    }
    let mut indices = Vec::new();
    let mut draws = Vec::new();
    for (image, list) in by_image {
        draws.push(ActorDraw { image, first_index: indices.len() as u32, index_count: list.len() as u32 });
        indices.extend(list);
    }
    let bind_local = skeleton.bind_local();
    let mut weapon_bones: Vec<u32> = meshes
        .iter()
        .flat_map(|(_, m)| m.vertices.iter().flat_map(|v| v.influences.iter().map(|i| i.bone)))
        .filter(|&b| b >= ATTACH_FIRST && (b as usize) < bones)
        .map(u32::from)
        .collect();
    weapon_bones.sort_unstable();
    weapon_bones.dedup();
    Ok(ActorModel { skeleton, bind_local, anims, vertices, indices, draws, images, weapon_bones, engaged: false })
}

/// Appends the actor's images to the scene's texture slots (same mip path as zone textures) and
/// returns the SRV slot of each image (the model's images are moved into the scene).
pub fn register_textures(model: &mut ActorModel, scene: &mut crate::scene::Scene) -> Vec<usize> {
    use crate::mips::{TexClass, classify};
    std::mem::take(&mut model.images)
        .into_iter()
        .map(|img| {
            let idx = scene.images.len();
            let class = if classify(&img.rgba) == TexClass::Opaque { TexClass::Opaque } else { TexClass::Cutout };
            scene.images.push(img);
            scene.slots.push((idx, class));
            scene.slots.len()
        })
        .collect()
}

impl ActorModel {
    /// True when a weapon mesh is loaded.
    pub fn armed(&self) -> bool {
        !self.weapon_bones.is_empty()
    }

    /// Idle animation: the weapon set's `idl1` (sheath mounts keyed) when armed and loaded.
    pub fn idle_name(&self) -> &'static str {
        if self.armed() && self.animation("idl1").is_some() { "idl1" } else { "idl0" }
    }

    /// Run animation, same rule as [`idle_name`](Self::idle_name).
    pub fn run_name(&self) -> &'static str {
        if self.armed() && self.animation("run1").is_some() { "run1" } else { "run0" }
    }

    pub fn animation(&self, name: &str) -> Option<&anim::Animation> {
        self.anims.iter().find(|(n, _)| n == name).map(|(_, a)| a)
    }

    /// Bounds of the skinned bind pose (min, max), y down.
    pub fn bind_extent(&self) -> ([f32; 3], [f32; 3]) {
        let pal = palette(&self.skeleton.bind_world());
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &self.vertices {
            let (p, _) = skin_vertex(v, &pal);
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        (lo, hi)
    }
}

/// Spherical interpolation of unit quaternions (x, y, z, w), taking the short way round.
pub fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let mut d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let mut b = b;
    if d < 0.0 {
        d = -d;
        b = b.map(|x| -x);
    }
    if d > 0.9995 {
        // Nearly parallel: normalized lerp avoids dividing by a tiny sine.
        return quat_normalize([0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t));
    }
    let th = d.clamp(-1.0, 1.0).acos();
    let s = th.sin();
    let (wa, wb) = (((1.0 - t) * th).sin() / s, (t * th).sin() / s);
    quat_normalize([0, 1, 2, 3].map(|i| a[i] * wa + b[i] * wb))
}

pub fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// Local bone poses at `seconds` into a looping animation: keyframes are sampled at the two
/// whole frames around the time, rotations are slerped and translations lerped.
pub fn local_poses_at(a: &anim::Animation, bind_local: &[Pose], seconds: f32) -> Vec<Pose> {
    let n = (a.frames as usize).max(1);
    let f = (seconds * FRAME_RATE * a.speed).rem_euclid(n as f32);
    let i0 = (f.floor() as usize).min(n - 1);
    let i1 = (i0 + 1) % n;
    let t = f - i0 as f32;
    let mut out = bind_local.to_vec();
    for tr in &a.tracks {
        if tr.skip || tr.bone >= out.len() {
            continue;
        }
        let (r0, t0, _) = tr.sample(i0 as f32);
        let (r1, t1, _) = tr.sample(i1 as f32);
        let (r, tt) = (slerp(r0, r1, t), lerp3(t0, t1, t));
        let bind = bind_local[tr.bone];
        out[tr.bone] = Pose {
            rotation: signet_dat::skeleton::quat_mul(r, bind.rotation),
            translation: [bind.translation[0] + tt[0], bind.translation[1] + tt[1], bind.translation[2] + tt[2]],
        };
    }
    out
}

/// The actor's placement in the zone: rotation about y plus a translation.
pub fn placement(position: [f32; 3], yaw_deg: f32) -> Pose {
    // Bind pose faces +x; yaw 0 faces +z like the camera, so the model turns by yaw - 90.
    let h = (yaw_deg - 90.0).to_radians() * 0.5;
    Pose { rotation: [0.0, h.sin(), 0.0, h.cos()], translation: position }
}

/// Palette entry: row-major 3x4 (rotation rows, translation in the last column).
pub type Bone3x4 = [f32; 12];

pub fn to_3x4(p: &Pose) -> Bone3x4 {
    let [x, y, z, w] = p.rotation;
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz, wx, wy, wz) = (x * y, x * z, y * z, w * x, w * y, w * z);
    [
        1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy), p.translation[0],
        2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx), p.translation[1],
        2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy), p.translation[2],
    ]
}

/// Bone palette from world poses; one identity entry is appended for out-of-range bones.
pub fn palette(world: &[Pose]) -> Vec<Bone3x4> {
    world.iter().map(to_3x4).chain(std::iter::once(to_3x4(&Pose::IDENTITY))).collect()
}

fn mul3(m: &Bone3x4, v: [f32; 3]) -> [f32; 3] {
    [0, 4, 8].map(|r| m[r] * v[0] + m[r + 1] * v[1] + m[r + 2] * v[2])
}

/// CPU reference of the vertex shader's skinning: (position, normal).
pub fn skin_vertex(v: &ActorVertex, pal: &[Bone3x4]) -> ([f32; 3], [f32; 3]) {
    let mut pos = [0.0f32; 3];
    let mut nrm = [0.0f32; 3];
    for (p, n, w, b) in [(v.p0, v.n0, v.w[0], v.bone[0]), (v.p1, v.n1, v.w[1], v.bone[1])] {
        let m = &pal[(b as usize).min(pal.len() - 1)];
        let (rp, rn) = (mul3(m, p), mul3(m, n));
        let t = [m[3], m[7], m[11]];
        for k in 0..3 {
            pos[k] += rp[k] + t[k] * w;
            nrm[k] += rn[k];
        }
    }
    let l = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
    if l > 1e-6 {
        nrm = nrm.map(|x| x / l);
    }
    (pos, nrm)
}

/// Nearest ground below `from_y` at (x, z): the triangle hit with the smallest y that is at or
/// below `from_y` (FFXI y points down). `None` when nothing is under that point.
pub fn ground_below(vertices: &[crate::scene::Vertex], indices: &[u32], x: f32, z: f32, from_y: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for t in indices.chunks_exact(3) {
        let p = [0, 1, 2].map(|k| vertices[t[k] as usize].pos);
        // Barycentric test in the xz plane.
        let d = (p[1][2] - p[2][2]) * (p[0][0] - p[2][0]) + (p[2][0] - p[1][0]) * (p[0][2] - p[2][2]);
        if d.abs() < 1e-9 {
            continue;
        }
        let a = ((p[1][2] - p[2][2]) * (x - p[2][0]) + (p[2][0] - p[1][0]) * (z - p[2][2])) / d;
        let b = ((p[2][2] - p[0][2]) * (x - p[2][0]) + (p[0][0] - p[2][0]) * (z - p[2][2])) / d;
        let c = 1.0 - a - b;
        if a < 0.0 || b < 0.0 || c < 0.0 {
            continue;
        }
        let y = a * p[0][1] + b * p[1][1] + c * p[2][1];
        if y >= from_y && best.is_none_or(|b| y < b) {
            best = Some(y);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use signet_dat::skeleton::{quat_mul, quat_rotate};

    fn q_axis(axis: [f32; 3], deg: f32) -> [f32; 4] {
        let h = deg.to_radians() * 0.5;
        [axis[0] * h.sin(), axis[1] * h.sin(), axis[2] * h.sin(), h.cos()]
    }
    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
    }

    #[test]
    fn slerp_endpoints_and_midpoint() {
        let a = q_axis([0.0, 1.0, 0.0], 0.0);
        let b = q_axis([0.0, 1.0, 0.0], 90.0);
        assert!(close(quat_rotate(slerp(a, b, 0.0), [1.0, 0.0, 0.0]), [1.0, 0.0, 0.0]));
        assert!(close(quat_rotate(slerp(a, b, 1.0), [1.0, 0.0, 0.0]), [0.0, 0.0, -1.0]));
        let m = slerp(a, b, 0.5);
        let want = q_axis([0.0, 1.0, 0.0], 45.0);
        assert!((0..4).all(|i| (m[i] - want[i]).abs() < 1e-5));
        // Constant angular speed: a quarter of the way is 22.5 degrees.
        let q = slerp(a, b, 0.25);
        let want = q_axis([0.0, 1.0, 0.0], 22.5);
        assert!((0..4).all(|i| (q[i] - want[i]).abs() < 1e-5));
    }

    #[test]
    fn slerp_takes_the_short_way() {
        let a = q_axis([0.0, 0.0, 1.0], 10.0);
        let b = a.map(|x| -x); // same rotation, opposite sign
        let m = slerp(a, b, 0.5);
        assert!((0..4).all(|i| (m[i] - a[i]).abs() < 1e-5), "{m:?}");
    }

    #[test]
    fn palette_matches_pose_apply() {
        let p = Pose { rotation: quat_normalize([0.3, -0.5, 0.2, 0.8]), translation: [1.0, 2.0, 3.0] };
        let m = to_3x4(&p);
        let v = [0.7, -1.1, 2.5];
        let r = mul3(&m, v);
        let got = [r[0] + m[3], r[1] + m[7], r[2] + m[11]];
        assert!(close(got, p.apply(v)));
    }

    #[test]
    fn skinning_matches_dat_reference() {
        // Two bones, a single-influence vertex and a dual (pre-weighted) vertex, normal and mirrored.
        let world = [
            Pose { rotation: q_axis([0.0, 1.0, 0.0], 30.0), translation: [1.0, 0.0, 0.0] },
            Pose { rotation: quat_mul(q_axis([1.0, 0.0, 0.0], 70.0), q_axis([0.0, 0.0, 1.0], -20.0)), translation: [0.0, -2.0, 0.5] },
        ];
        let inf = |bone, bone_mirror, axis, weight: f32, pos: [f32; 3]| actor_mesh::Influence {
            bone,
            bone_mirror,
            mirror_axis: axis,
            weight,
            position: pos.map(|x| x * weight),
            normal: [0.0, 0.6, 0.8],
        };
        let mesh = actor_mesh::ActorMesh {
            cloth: false,
            flip: true,
            vertices: vec![
                actor_mesh::SkinVertex { influences: vec![inf(1, 0, 1, 1.0, [0.3, 0.2, -0.4])] },
                actor_mesh::SkinVertex { influences: vec![inf(0, 1, 3, 0.25, [0.5, 0.1, 0.9]), inf(1, 0, 2, 0.75, [0.5, 0.1, 0.9])] },
            ],
            groups: vec![],
        };
        let pal = palette(&world);
        for mirrored in [false, true] {
            let reference = mesh.skin(&world, mirrored);
            for (v, r) in mesh.vertices.iter().zip(&reference) {
                let (p, n) = skin_vertex(&make_vertex(v, mirrored, world.len()), &pal);
                assert!(close(p, r.0), "pos {p:?} vs {:?}", r.0);
                assert!(close(n, r.1), "normal {n:?} vs {:?}", r.1);
            }
        }
    }

    #[test]
    fn out_of_range_bone_uses_identity() {
        let world = [Pose { rotation: q_axis([0.0, 1.0, 0.0], 90.0), translation: [5.0, 5.0, 5.0] }];
        let v = make_vertex(
            &actor_mesh::SkinVertex {
                influences: vec![actor_mesh::Influence { bone: 9, bone_mirror: 9, mirror_axis: 0, weight: 1.0, position: [1.0, 2.0, 3.0], normal: [0.0, 1.0, 0.0] }],
            },
            false,
            1,
        );
        assert!(close(skin_vertex(&v, &palette(&world)).0, [1.0, 2.0, 3.0]));
    }

    #[test]
    fn animation_interpolates_between_keyframes() {
        use anim::{Animation, Channel, Track};
        let one = Channel::Const;
        // Rotation about y: 0 deg on frame 0, 90 deg on frame 1; x translation 0 -> 4 -> 0.
        let h = 45f32.to_radians();
        let track = Track {
            bone: 0,
            skip: false,
            rotation: [one(0.0), Channel::Keys(vec![0.0, h.sin(), 0.0]), one(0.0), Channel::Keys(vec![1.0, h.cos(), 1.0])],
            translation: [Channel::Keys(vec![0.0, 4.0, 0.0]), one(0.0), one(0.0)],
            scale: [one(1.0), one(1.0), one(1.0)],
        };
        let a = Animation { frames: 3, speed: 1.0, tracks: vec![track] };
        let bind = [Pose::IDENTITY];
        // 0.5 frames in: half way, 45 degrees about y and x translation 2.
        let p = local_poses_at(&a, &bind, 0.5 / FRAME_RATE)[0];
        let want = q_axis([0.0, 1.0, 0.0], 45.0);
        assert!((0..4).all(|i| (p.rotation[i] - want[i]).abs() < 1e-4), "{:?}", p.rotation);
        assert!((p.translation[0] - 2.0).abs() < 1e-4);
        // Exactly on a key, and wrapping after the last frame.
        assert!((local_poses_at(&a, &bind, 1.0 / FRAME_RATE)[0].translation[0] - 4.0).abs() < 1e-4);
        assert!((local_poses_at(&a, &bind, 3.0 / FRAME_RATE)[0].translation[0]).abs() < 1e-4);
    }

    #[test]
    fn placement_faces_the_yaw_direction() {
        // Bind forward is +x; yaw 0 should face +z, yaw 90 should face +x.
        let f0 = placement([0.0; 3], 0.0).apply([1.0, 0.0, 0.0]);
        assert!(close(f0, [0.0, 0.0, 1.0]), "{f0:?}");
        let f90 = placement([1.0, 2.0, 3.0], 90.0).apply([1.0, 0.0, 0.0]);
        assert!(close(f90, [2.0, 2.0, 3.0]), "{f90:?}");
    }

    #[test]
    fn ground_ray_picks_nearest_below() {
        use crate::scene::Vertex;
        let v = |x: f32, y: f32, z: f32| Vertex { pos: [x, y, z], normal: [0.0, -1.0, 0.0], uv: [0.0; 2], color: 0 };
        let mut verts = vec![];
        let mut idx = vec![];
        for y in [10.0f32, 0.0] {
            let b = verts.len() as u32;
            verts.extend([v(-5.0, y, -5.0), v(5.0, y, -5.0), v(0.0, y, 5.0)]);
            idx.extend([b, b + 1, b + 2]);
        }
        // y points down: from above both, the smaller y (higher up) wins; from between, only the lower floor.
        assert_eq!(ground_below(&verts, &idx, 0.0, 0.0, -5.0), Some(0.0));
        assert_eq!(ground_below(&verts, &idx, 0.0, 0.0, 5.0), Some(10.0));
        assert_eq!(ground_below(&verts, &idx, 50.0, 0.0, -5.0), None);
    }
}

impl ActorModel {
    /// Bone palette for an animation at `seconds` (None: bind pose), placed in the zone.
    pub fn pose_palette(&self, anim: Option<&anim::Animation>, seconds: f32, place: &Pose) -> Vec<Bone3x4> {
        let local = match anim {
            Some(a) => local_poses_at(a, &self.bind_local, seconds),
            None => self.bind_local.clone(),
        };
        let mut skel = self.skeleton.world_from_local(&local);
        if self.engaged && GRIP_RIGHT < skel.len() {
            let grip = skel[GRIP_RIGHT];
            for &b in &self.weapon_bones {
                skel[b as usize] = grip;
            }
        }
        let world: Vec<Pose> = skel.iter().map(|w| place.compose(w)).collect();
        palette(&world)
    }
}
