//! Zone collision from a decoded MZB payload (see `crypt::decode_mzb`).
//!
//! Layout found by inspecting decoded zone layouts (original work, no outside code):
//! - Header u32 at byte 8 is the offset of a 32-byte collision header of eight u32s:
//!   `[mesh count, first mesh offset, non-empty cell count, cell lists offset,
//!   grid offset, instance table offset, instance count, 0]`.
//! - Instance table: one entry per placement record, 192 bytes (112 in older, plain-stored
//!   zones). The first 64 bytes are a 4x4 float matrix (row vectors, translation in the
//!   last row) taking mesh-local points to world space. In the 192-byte form the next 64
//!   are its inverse. The rest is unread. The cell lists start right after the table.
//! - Collision mesh: 16-byte header `[vertex off, normal off, triangle off, count word]`
//!   (low u16 = triangle count, high u16 = unknown, 0 or 1), then vertices and normals (3 f32 each) and 8-byte triangles
//!   `[a, b, c, normal index]` as u16. The top two bits of each index are flags (kept in
//!   `Tri::flags`, meaning unknown); the low 14 bits are the vertex index.
//! - Grid: an array of u32 cell offsets (0 = empty), 4 yalms per cell, centered on the
//!   origin (to within about a yalm; the engine's exact origin is not located). Header
//!   bytes 12 and 13 times bytes 14 and 15 give the span in yalms (x, z): 101 has
//!   50 * 40 = 2000, so 500 cells covering -1000..1000. Cell x is the column,
//!   cell z the row. A cell offset points at `[key, (instance, mesh)..., 0]` in the
//!   cell-lists area. Instance and mesh values are absolute payload offsets.
//! - The origin is exactly -span/2 (checked on 101: 3333 of 3333 small meshes fit that origin
//!   best). The gaps came from the lists: each (instance, mesh) patch is listed in exactly ONE
//!   cell, yet its triangles spill past that cell's border (up to ~4 yalms; e.g. 101 at
//!   x 141, z -8.0..-7.97). Lookups therefore use `query_cells`, rebuilt from the geometry.
//!
//! FFXI's y axis points down, so the highest surface has the smallest y.

use std::collections::HashMap;

/// Yalms per grid cell.
pub const CELL: f32 = 4.0;
/// Instance entry sizes seen: newer zones 192 bytes, older (MZB version < 0x1b) 112.
const STRIDES: [usize; 2] = [192, 112];
const IDX_MASK: u16 = 0x3fff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollisionError {
    /// Payload too short for the header or the collision block offset is not set.
    NoCollision,
    /// A table offset or size runs outside the payload.
    BadHeader(&'static str),
    /// A cell list entry points outside the instance table or mesh area, or never ends.
    BadCell { cell: usize, why: &'static str },
    /// A mesh is malformed (offsets, indices or normal index out of range).
    BadMesh { offset: u32, why: &'static str },
}

impl std::fmt::Display for CollisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCollision => write!(f, "no collision block"),
            Self::BadHeader(w) => write!(f, "bad header: {w}"),
            Self::BadCell { cell, why } => write!(f, "bad cell {cell}: {why}"),
            Self::BadMesh { offset, why } => write!(f, "bad mesh at {offset:#x}: {why}"),
        }
    }
}
impl std::error::Error for CollisionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tri {
    /// Vertex indices (flag bits removed).
    pub idx: [u16; 3],
    /// Top two bits of each corner's index (0..=3): bit 15 = 2, bit 14 = 1.
    pub flags: [u8; 3],
    pub normal: u16,
    /// Top two bits of the normal word (same kind of flags as `flags`).
    pub normal_flags: u8,
}

#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tris: Vec<Tri>,
    /// High half of the triangle-count word (0 or 1 seen; meaning unknown).
    pub extra: u16,
}

/// Mesh-local to world: `p' = p * rows + t` (three rows, then translation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub rows: [[f32; 3]; 3],
    pub t: [f32; 3],
}

impl Transform {
    pub fn point(&self, p: [f32; 3]) -> [f32; 3] {
        let r = &self.rows;
        [
            p[0] * r[0][0] + p[1] * r[1][0] + p[2] * r[2][0] + self.t[0],
            p[0] * r[0][1] + p[1] * r[1][1] + p[2] * r[2][1] + self.t[1],
            p[0] * r[0][2] + p[1] * r[1][2] + p[2] * r[2][2] + self.t[2],
        ]
    }
}

/// One placed collision mesh (a placement record's transform and a mesh).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Placed {
    /// Index of the placement record (same order as `crypt::placements`).
    pub instance: u32,
    pub mesh: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Distance along the (unit) direction.
    pub t: f32,
    pub point: [f32; 3],
    /// Unit triangle normal, flipped to face the ray origin.
    pub normal: [f32; 3],
    pub placed: u32,
    pub tri: u32,
    pub flags: [u8; 3],
}

pub struct Collision {
    pub meshes: Vec<Mesh>,
    /// Transform per placement record.
    pub instances: Vec<Transform>,
    /// Distinct (instance, mesh) pairs referenced by the grid.
    pub placed: Vec<Placed>,
    /// Grid columns (x) and rows (z).
    pub cols: usize,
    pub rows: usize,
    /// World x and z of the grid's corner.
    pub origin: [f32; 2],
    /// Per cell: (start, len) into `cell_items`.
    cells: Vec<(u32, u32)>,
    /// Indices into `placed`.
    cell_items: Vec<u32>,
    /// Conservative query index, rebuilt from the geometry: per cell (start, len) into `query_items`,
    /// listing every placed mesh with a triangle within `QUERY_EPS` of the cell. See `build_query_index`.
    query_cells: Vec<(u32, u32)>,
    query_items: Vec<u32>,
}

/// Slack in yalms added around each cell when deciding which placed meshes a cell can see.
const QUERY_EPS: f32 = 0.02;

/// 2D overlap of a triangle (x/z of `w`) with the rectangle `lo..hi` (separating axes).
fn tri_overlaps_rect(w: &[[f32; 3]; 3], lo: [f32; 2], hi: [f32; 2]) -> bool {
    let p = w.map(|q| [q[0], q[2]]);
    for k in 0..2 {
        if p.iter().all(|q| q[k] < lo[k]) || p.iter().all(|q| q[k] > hi[k]) {
            return false;
        }
    }
    let r = [[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]];
    for i in 0..3 {
        let (a, b) = (p[i], p[(i + 1) % 3]);
        let n = [b[1] - a[1], a[0] - b[0]];
        let d = |q: [f32; 2]| (q[0] - a[0]) * n[0] + (q[1] - a[1]) * n[1];
        let (tmin, tmax) = p.iter().map(|&q| d(q)).fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v), b.max(v)));
        let (rmin, rmax) = r.iter().map(|&q| d(q)).fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v), b.max(v)));
        if rmin > tmax || rmax < tmin {
            return false;
        }
    }
    true
}

fn u32_at(p: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(p.get(i..i + 4)?.try_into().ok()?))
}
fn f32_at(p: &[u8], i: usize) -> Option<f32> {
    u32_at(p, i).map(f32::from_bits)
}

fn parse_mesh(p: &[u8], off: u32) -> Result<Mesh, CollisionError> {
    let bad = |why| CollisionError::BadMesh { offset: off, why };
    let o = off as usize;
    let rd = |k: usize| u32_at(p, o + 4 * k).map(|v| v as usize).ok_or(bad("header"));
    let (vo, no, to, tc_word) = (rd(0)?, rd(1)?, rd(2)?, rd(3)?);
    let (tc, extra) = (tc_word & 0xffff, (tc_word >> 16) as u16);
    if !(vo <= no && no <= to) || (no - vo) % 12 != 0 || (to - no) % 12 != 0 {
        return Err(bad("table order"));
    }
    if tc.checked_mul(8).and_then(|n| n.checked_add(to)).is_none_or(|e| e > p.len()) {
        return Err(bad("table past end"));
    }
    let vec3 = |a: usize, b: usize| -> Vec<[f32; 3]> {
        (a..b).step_by(12).map(|i| [f32_at(p, i).unwrap(), f32_at(p, i + 4).unwrap(), f32_at(p, i + 8).unwrap()]).collect()
    };
    let vertices = vec3(vo, no);
    let normals = vec3(no, to);
    let mut tris = Vec::with_capacity(tc);
    for k in 0..tc {
        let at = to + k * 8;
        let w = |j: usize| u16::from_le_bytes([p[at + 2 * j], p[at + 2 * j + 1]]);
        let raw = [w(0), w(1), w(2)];
        let idx = raw.map(|r| r & IDX_MASK);
        let flags = raw.map(|r| (r >> 14) as u8);
        let (normal, normal_flags) = (w(3) & IDX_MASK, (w(3) >> 14) as u8);
        if idx.iter().any(|&i| i as usize >= vertices.len()) {
            return Err(bad("vertex index"));
        }
        if normal as usize >= normals.len() {
            return Err(bad("normal index"));
        }
        tris.push(Tri { idx, flags, normal, normal_flags });
    }
    Ok(Mesh { vertices, normals, tris, extra })
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l > 0.0 { [a[0] / l, a[1] / l, a[2] / l] } else { [0.0, -1.0, 0.0] }
}

impl Collision {
    /// Reads collision from a decoded MZB payload.
    pub fn parse(p: &[u8]) -> Result<Collision, CollisionError> {
        use CollisionError::*;
        if p.len() < 40 {
            return Err(NoCollision);
        }
        let block = u32_at(p, 8).unwrap() as usize;
        if block == 0 {
            return Err(NoCollision);
        }
        if block + 32 > p.len() {
            return Err(BadHeader("block offset"));
        }
        let h: Vec<usize> = (0..8).map(|k| u32_at(p, block + 4 * k).unwrap() as usize).collect();
        let (mesh_start, lists, grid, inst_at, n_inst) = (h[1], h[3], h[4], h[5], h[6]);
        let records = p[4] as usize | (p[5] as usize) << 8 | (p[6] as usize) << 16;
        if n_inst != records {
            return Err(BadHeader("instance count differs from placement count"));
        }
        if inst_at > lists || lists > grid || grid > p.len() || n_inst == 0 {
            return Err(BadHeader("table order"));
        }
        // The cell lists follow the instance table directly, so the stride is the gap / count.
        let gap = lists - inst_at;
        let Some(stride) = STRIDES.into_iter().find(|s| gap == n_inst * s) else {
            return Err(BadHeader("instance table size"));
        };
        if mesh_start > inst_at {
            return Err(BadHeader("mesh start"));
        }
        // Header bytes 12/13 times bytes 14/15 give the grid span in yalms (x, z).
        let (cols, rows) = (p[12] as usize * p[14] as usize / 4, p[13] as usize * p[15] as usize / 4);
        if cols == 0 || rows == 0 || (p.len() - grid) / 4 < cols * rows {
            return Err(BadHeader("grid size"));
        }

        let mut instances = Vec::with_capacity(n_inst);
        for i in 0..n_inst {
            let at = inst_at + i * stride;
            let f = |k: usize| f32_at(p, at + 4 * k).unwrap();
            instances.push(Transform {
                rows: [[f(0), f(1), f(2)], [f(4), f(5), f(6)], [f(8), f(9), f(10)]],
                t: [f(12), f(13), f(14)],
            });
        }

        let mut meshes: Vec<Mesh> = Vec::new();
        let mut mesh_by_off: HashMap<u32, u32> = HashMap::new();
        let mut placed: Vec<Placed> = Vec::new();
        let mut placed_by: HashMap<(u32, u32), u32> = HashMap::new();
        let mut cells = vec![(0u32, 0u32); cols * rows];
        let mut cell_items: Vec<u32> = Vec::new();
        for (c, cell) in cells.iter_mut().enumerate() {
            let ptr = u32_at(p, grid + 4 * c).unwrap() as usize;
            if ptr == 0 {
                continue;
            }
            if ptr < lists || ptr + 4 > p.len() {
                return Err(BadCell { cell: c, why: "list offset" });
            }
            let start = cell_items.len() as u32;
            let mut q = ptr + 4; // skip the key
            loop {
                let Some(inst) = u32_at(p, q) else { return Err(BadCell { cell: c, why: "unterminated" }) };
                if inst == 0 {
                    break;
                }
                let Some(mesh) = u32_at(p, q + 4) else { return Err(BadCell { cell: c, why: "unterminated" }) };
                q += 8;
                let io = inst as usize;
                if io < inst_at || (io - inst_at) % stride != 0 || (io - inst_at) / stride >= n_inst {
                    return Err(BadCell { cell: c, why: "instance offset" });
                }
                if (mesh as usize) < mesh_start || mesh as usize >= inst_at {
                    return Err(BadCell { cell: c, why: "mesh offset" });
                }
                let mi = match mesh_by_off.get(&mesh) {
                    Some(&m) => m,
                    None => {
                        meshes.push(parse_mesh(p, mesh)?);
                        mesh_by_off.insert(mesh, meshes.len() as u32 - 1);
                        meshes.len() as u32 - 1
                    }
                };
                let key = (((io - inst_at) / stride) as u32, mi);
                let pi = *placed_by.entry(key).or_insert_with(|| {
                    placed.push(Placed { instance: key.0, mesh: key.1 });
                    placed.len() as u32 - 1
                });
                cell_items.push(pi);
            }
            *cell = (start, cell_items.len() as u32 - start);
        }
        let origin = [-(cols as f32) * CELL / 2.0, -(rows as f32) * CELL / 2.0];
        let mut col = Collision { meshes, instances, placed, cols, rows, origin, cells, cell_items, query_cells: Vec::new(), query_items: Vec::new() };
        col.build_query_index();
        Ok(col)
    }

    /// The file lists each placed mesh in exactly one cell (a spatial partition of small patches),
    /// but a patch's triangles spill past that cell's border (by up to a few yalms in 101).
    /// Looking only in the point's own cell therefore misses surfaces that belong to a neighbor's
    /// list. This builds the index lookups actually use: every placed mesh goes into every cell
    /// that any of its triangles overlaps (cell grown by `QUERY_EPS`), plus the cell the file listed.
    fn build_query_index(&mut self) {
        let mut per_cell: Vec<Vec<u32>> = vec![Vec::new(); self.cols * self.rows];
        for (c, items) in (0..self.cells.len()).map(|c| (c, self.cell_items(c))) {
            for &pi in items {
                per_cell[c].push(pi);
            }
        }
        let max_c = [self.cols as i64 - 1, self.rows as i64 - 1];
        for (pi, pl) in self.placed.iter().enumerate() {
            let m = &self.meshes[pl.mesh as usize];
            let t = &self.instances[pl.instance as usize];
            for tr in &m.tris {
                let w = tr.idx.map(|i| t.point(m.vertices[i as usize]));
                let mut lo = [f32::MAX; 2];
                let mut hi = [f32::MIN; 2];
                for q in &w {
                    for (k, a) in [0usize, 2].into_iter().enumerate() {
                        lo[k] = lo[k].min(q[a]);
                        hi[k] = hi[k].max(q[a]);
                    }
                }
                if !(lo[0].is_finite() && hi[0].is_finite() && lo[1].is_finite() && hi[1].is_finite()) {
                    continue;
                }
                let c0 = |k: usize| (((lo[k] - QUERY_EPS - self.origin[k]) / CELL).floor() as i64).max(0);
                let c1 = |k: usize| (((hi[k] + QUERY_EPS - self.origin[k]) / CELL).floor() as i64).min(max_c[k]);
                for cz in c0(1)..=c1(1) {
                    for cx in c0(0)..=c1(0) {
                        let (x0, z0) = (self.origin[0] + cx as f32 * CELL, self.origin[1] + cz as f32 * CELL);
                        if !tri_overlaps_rect(&w, [x0 - QUERY_EPS, z0 - QUERY_EPS], [x0 + CELL + QUERY_EPS, z0 + CELL + QUERY_EPS]) {
                            continue;
                        }
                        let list = &mut per_cell[cz as usize * self.cols + cx as usize];
                        // placed meshes are visited in order, so a repeat can only be the last entry
                        if list.last() != Some(&(pi as u32)) && !list.contains(&(pi as u32)) {
                            list.push(pi as u32);
                        }
                    }
                }
            }
        }
        self.query_cells = Vec::with_capacity(per_cell.len());
        self.query_items = Vec::new();
        for l in per_cell {
            self.query_cells.push((self.query_items.len() as u32, l.len() as u32));
            self.query_items.extend(l);
        }
    }

    fn query_items(&self, c: usize) -> &[u32] {
        let (s, n) = self.query_cells[c];
        &self.query_items[s as usize..(s + n) as usize]
    }

    pub fn triangle_count(&self) -> usize {
        self.placed.iter().map(|pl| self.meshes[pl.mesh as usize].tris.len()).sum()
    }

    /// World-space corners of triangle `tri` of placed mesh `placed`.
    pub fn world_tri(&self, placed: u32, tri: u32) -> [[f32; 3]; 3] {
        let pl = self.placed[placed as usize];
        let m = &self.meshes[pl.mesh as usize];
        let t = &self.instances[pl.instance as usize];
        m.tris[tri as usize].idx.map(|i| t.point(m.vertices[i as usize]))
    }

    /// Calls `f(placed index, triangle index, world corners, flags)` for every distinct triangle.
    pub fn for_each_triangle(&self, mut f: impl FnMut(u32, u32, [[f32; 3]; 3], [u8; 3])) {
        for (pi, pl) in self.placed.iter().enumerate() {
            let m = &self.meshes[pl.mesh as usize];
            let t = &self.instances[pl.instance as usize];
            for (ti, tr) in m.tris.iter().enumerate() {
                f(pi as u32, ti as u32, tr.idx.map(|i| t.point(m.vertices[i as usize])), tr.flags);
            }
        }
    }

    /// Grid cell (column, row) containing world (x, z).
    pub fn cell_of(&self, x: f32, z: f32) -> Option<(usize, usize)> {
        let cx = ((x - self.origin[0]) / CELL).floor();
        let cz = ((z - self.origin[1]) / CELL).floor();
        if !(cx >= 0.0 && cz >= 0.0 && cx < self.cols as f32 && cz < self.rows as f32) {
            return None;
        }
        Some((cx as usize, cz as usize))
    }

    fn cell_items(&self, c: usize) -> &[u32] {
        let (s, n) = self.cells[c];
        &self.cell_items[s as usize..(s + n) as usize]
    }

    /// Non-empty cells as `(cell index = row * cols + column, placed indices)`.
    pub fn non_empty_cells(&self) -> impl Iterator<Item = (usize, &[u32])> {
        (0..self.cells.len()).filter(|&c| self.cells[c].1 > 0).map(|c| (c, self.cell_items(c)))
    }

    /// Every surface height (y values, smallest first = highest in the world) under (x, z).
    pub fn surfaces_at(&self, x: f32, z: f32) -> Vec<f32> {
        let Some((cx, cz)) = self.cell_of(x, z) else { return Vec::new() };
        let mut out = Vec::new();
        for &pi in self.query_items(cz * self.cols + cx) {
            let pl = self.placed[pi as usize];
            let m = &self.meshes[pl.mesh as usize];
            let t = &self.instances[pl.instance as usize];
            for tr in &m.tris {
                let w = tr.idx.map(|i| t.point(m.vertices[i as usize]));
                if let Some(y) = height_in_tri(&w, x, z) {
                    out.push(y);
                }
            }
        }
        out.sort_by(|a, b| a.total_cmp(b));
        out.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        out
    }

    /// Diagnostic: highest surface using only the meshes the file lists in the point's own cell
    /// (the pre-fix lookup). Misses surfaces whose mesh is listed in a neighboring cell.
    pub fn ground_height_listed_only(&self, x: f32, z: f32) -> Option<f32> {
        let (cx, cz) = self.cell_of(x, z)?;
        let mut best: Option<f32> = None;
        for &pi in self.cell_items(cz * self.cols + cx) {
            let pl = self.placed[pi as usize];
            let m = &self.meshes[pl.mesh as usize];
            let t = &self.instances[pl.instance as usize];
            for tr in &m.tris {
                let w = tr.idx.map(|i| t.point(m.vertices[i as usize]));
                if let Some(y) = height_in_tri(&w, x, z) {
                    best = Some(best.map_or(y, |b| b.min(y)));
                }
            }
        }
        best
    }

    /// Highest surface at (x, z): the smallest y (y points down in FFXI).
    pub fn ground_height(&self, x: f32, z: f32) -> Option<f32> {
        self.surfaces_at(x, z).first().copied()
    }

    /// First surface at or below `y` (allows 0.5 yalm of sink so a standing actor finds its floor).
    pub fn ground_height_below(&self, x: f32, y: f32, z: f32) -> Option<f32> {
        self.surfaces_at(x, z).into_iter().find(|&s| s >= y - 0.5)
    }

    /// Nearest hit along `origin + t * dir` for t in 0..=max (`dir` need not be unit).
    pub fn raycast(&self, origin: [f32; 3], dir: [f32; 3], max: f32) -> Option<Hit> {
        let len = dot(dir, dir).sqrt();
        if len == 0.0 || !max.is_finite() {
            return None;
        }
        let d = [dir[0] / len, dir[1] / len, dir[2] / len];
        let mut best: Option<Hit> = None;
        let test_cell = |c: usize, best: &mut Option<Hit>| {
            for &pi in self.query_items(c) {
                let pl = self.placed[pi as usize];
                let m = &self.meshes[pl.mesh as usize];
                let tf = &self.instances[pl.instance as usize];
                for (ti, tr) in m.tris.iter().enumerate() {
                    let w = tr.idx.map(|i| tf.point(m.vertices[i as usize]));
                    if let Some(t) = ray_tri(origin, d, &w) {
                        if t <= max && best.is_none_or(|b| t < b.t) {
                            let mut n = normalize(cross(sub(w[1], w[0]), sub(w[2], w[0])));
                            if dot(n, d) > 0.0 {
                                n = [-n[0], -n[1], -n[2]];
                            }
                            *best = Some(Hit {
                                t,
                                point: [origin[0] + d[0] * t, origin[1] + d[1] * t, origin[2] + d[2] * t],
                                normal: n,
                                placed: pi,
                                tri: ti as u32,
                                flags: tr.flags,
                            });
                        }
                    }
                }
            }
        };
        // Grid walk in x/z (Amanatides-Woo), clipped to the grid square.
        let span_xz = [self.cols as f32 * CELL, self.rows as f32 * CELL];
        let (mut t0, mut t1) = (0.0f32, max);
        for k in [0usize, 2] {
            let span = span_xz[k / 2];
            let (o, dd) = (origin[k] - self.origin[k / 2], d[k]);
            if dd.abs() < 1e-9 {
                if o < 0.0 || o >= span {
                    return None;
                }
            } else {
                let (a, b) = ((0.0 - o) / dd, (span - o) / dd);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
        }
        if t0 > t1 {
            return None;
        }
        let eps = 1e-4;
        let (sx, sz) = (origin[0] + d[0] * (t0 + eps), origin[2] + d[2] * (t0 + eps));
        let mut cx = (((sx - self.origin[0]) / CELL).floor() as i64).clamp(0, self.cols as i64 - 1);
        let mut cz = (((sz - self.origin[1]) / CELL).floor() as i64).clamp(0, self.rows as i64 - 1);
        let step = |dd: f32| if dd > 0.0 { 1i64 } else { -1 };
        let (stx, stz) = (step(d[0]), step(d[2]));
        let next_t = |c: i64, st: i64, o: f32, dd: f32, org: f32| -> f32 {
            if dd.abs() < 1e-9 {
                f32::INFINITY
            } else {
                let edge = org + (c + if st > 0 { 1 } else { 0 }) as f32 * CELL;
                (edge - o) / dd
            }
        };
        let (mut tx, mut tz) = (next_t(cx, stx, origin[0], d[0], self.origin[0]), next_t(cz, stz, origin[2], d[2], self.origin[1]));
        let (dtx, dtz) = (
            if d[0].abs() < 1e-9 { f32::INFINITY } else { CELL / d[0].abs() },
            if d[2].abs() < 1e-9 { f32::INFINITY } else { CELL / d[2].abs() },
        );
        loop {
            test_cell(cz as usize * self.cols + cx as usize, &mut best);
            let exit = tx.min(tz);
            if best.is_some_and(|b| b.t <= exit) || exit > t1 {
                break;
            }
            if tx < tz {
                cx += stx;
                tx += dtx;
            } else {
                cz += stz;
                tz += dtz;
            }
            if cx < 0 || cz < 0 || cx >= self.cols as i64 || cz >= self.rows as i64 {
                break;
            }
        }
        best
    }
}

/// y of triangle `w` at (x, z) when the point lies inside its x/z projection.
fn height_in_tri(w: &[[f32; 3]; 3], x: f32, z: f32) -> Option<f32> {
    let area = (w[1][0] - w[0][0]) * (w[2][2] - w[0][2]) - (w[2][0] - w[0][0]) * (w[1][2] - w[0][2]);
    if area.abs() < 1e-9 {
        return None;
    }
    let l1 = ((x - w[0][0]) * (w[2][2] - w[0][2]) - (w[2][0] - w[0][0]) * (z - w[0][2])) / area;
    let l2 = ((w[1][0] - w[0][0]) * (z - w[0][2]) - (x - w[0][0]) * (w[1][2] - w[0][2])) / area;
    let l0 = 1.0 - l1 - l2;
    let e = -1e-5;
    if l0 < e || l1 < e || l2 < e {
        return None;
    }
    Some(l0 * w[0][1] + l1 * w[1][1] + l2 * w[2][1])
}

/// Two-sided Moller-Trumbore; returns t >= 0.
fn ray_tri(o: [f32; 3], d: [f32; 3], w: &[[f32; 3]; 3]) -> Option<f32> {
    let (e1, e2) = (sub(w[1], w[0]), sub(w[2], w[0]));
    let pv = cross(d, e2);
    let det = dot(e1, pv);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = sub(o, w[0]);
    let u = dot(tv, pv) * inv;
    if !(-1e-5..=1.0 + 1e-5).contains(&u) {
        return None;
    }
    let qv = cross(tv, e1);
    let v = dot(d, qv) * inv;
    if v < -1e-5 || u + v > 1.0 + 1e-5 {
        return None;
    }
    let t = dot(e2, qv) * inv;
    (t >= 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(b: &mut Vec<u8>, v: u32) {
        b.extend_from_slice(&v.to_le_bytes());
    }
    fn putf(b: &mut Vec<u8>, v: f32) {
        put(b, v.to_bits());
    }

    /// One placement (translated by (10, 5, 0)), one quad mesh at y = 0, 10 x 10 grid.
    fn synthetic() -> Vec<u8> {
        synthetic_with(192)
    }

    fn synthetic_with(stride: usize) -> Vec<u8> {
        let dim = 10usize;
        let mut b = vec![0u8; 32];
        b[4] = 1; // one placement record
        b.extend_from_slice(&[0u8; 100]);
        let block = b.len();
        b.extend_from_slice(&[0u8; 32]);
        let mesh_start = b.len();
        let mesh_at = b.len();
        let (vo, no, to) = (mesh_at + 16, mesh_at + 16 + 48, mesh_at + 16 + 48 + 12);
        for v in [vo, no, to, 2] {
            put(&mut b, v as u32);
        }
        for v in [[0., 0., 0.], [4., 0., 0.], [0., 0., 4.], [4., 0., 4.]] {
            v.iter().for_each(|&c| putf(&mut b, c));
        }
        [0.0f32, -1.0, 0.0].iter().for_each(|&c| putf(&mut b, c));
        for t in [[0x8000u16, 1, 2, 0], [1, 3, 2, 0]] {
            t.iter().for_each(|&c| b.extend_from_slice(&c.to_le_bytes()));
        }
        let inst_at = b.len();
        let mut m = vec![0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[12] = 10.0;
        m[13] = 5.0;
        m[15] = 1.0;
        m.iter().for_each(|&c| putf(&mut b, c));
        b.extend_from_slice(&vec![0u8; stride - 64]);
        let lists = b.len();
        // world x 10..14, z 0..4 with origin -20 => column 7, row 5
        let list_at = b.len();
        for v in [0xabcd, inst_at as u32, mesh_at as u32, 0] {
            put(&mut b, v);
        }
        let grid = b.len();
        let mut cells = vec![0u32; dim * dim];
        cells[5 * dim + 7] = list_at as u32;
        cells.iter().for_each(|&c| put(&mut b, c));
        for (k, v) in [1, mesh_start as u32, 1, lists as u32, grid as u32, inst_at as u32, 1, 0].into_iter().enumerate() {
            b[block + 4 * k..block + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[8..12].copy_from_slice(&(block as u32).to_le_bytes());
        b[12] = 1;
        b[13] = 1;
        b[14] = 40;
        b[15] = 40;
        b
    }

    #[test]
    fn parses_synthetic_zone() {
        let c = Collision::parse(&synthetic()).unwrap();
        assert_eq!(((c.cols, c.rows), c.meshes.len(), c.placed.len(), c.triangle_count()), ((10, 10), 1, 1, 2));
        assert_eq!(c.meshes[0].tris[0], Tri { idx: [0, 1, 2], flags: [2, 0, 0], normal: 0, normal_flags: 0 });
        assert_eq!(c.origin, [-20.0, -20.0]);
        assert_eq!(c.non_empty_cells().count(), 1);
    }

    #[test]
    fn parses_older_112_byte_instances() {
        let c = Collision::parse(&synthetic_with(112)).unwrap();
        assert!((c.ground_height(11.0, 1.0).unwrap() - 5.0).abs() < 1e-4);
        assert_eq!(Collision::parse(&synthetic_with(100)).err(), Some(CollisionError::BadHeader("instance table size")));
    }

    #[test]
    fn ground_height_and_raycast() {
        let c = Collision::parse(&synthetic()).unwrap();
        assert!((c.ground_height(11.0, 1.0).unwrap() - 5.0).abs() < 1e-4);
        assert!(c.ground_height(11.0, 5.0).is_none()); // outside the quad
        assert!(c.ground_height(-500.0, 0.0).is_none()); // outside the grid
        assert!(c.ground_height_below(11.0, 6.0, 1.0).is_none());
        assert!(c.ground_height_below(11.0, 4.0, 1.0).is_some());
        // y points down: fire from above (y = -10) downward (+y)
        let h = c.raycast([11.0, -10.0, 1.0], [0.0, 1.0, 0.0], 100.0).unwrap();
        assert!((h.t - 15.0).abs() < 1e-3 && h.normal[1] < 0.0);
        assert!(c.raycast([11.0, -10.0, 1.0], [0.0, 1.0, 0.0], 10.0).is_none());
        // edge-on ray misses; slanted ray crossing several cells hits at x = 10
        assert!(c.raycast([-5.0, 5.0, 2.0], [1.0, 0.0, 0.0], 100.0).is_none());
        let h = c.raycast([-10.0, -5.0, 2.0], [1.0, 0.5, 0.0], 100.0).unwrap();
        assert!((h.point[1] - 5.0).abs() < 1e-3 && (h.point[0] - 10.0).abs() < 1e-3);
    }

    /// A mesh listed in one cell whose triangles spill into the next (as in real zones) must still
    /// be found there, by height and by ray.
    #[test]
    fn mesh_spilling_past_its_listed_cell_is_found() {
        let mut c = Collision::parse(&synthetic()).unwrap();
        c.instances[0].t[0] = 11.9; // quad now spans x 11.9..15.9; listed in column 7 (x 10..14)
        c.build_query_index();
        assert!(c.ground_height_listed_only(15.0, 1.0).is_none(), "the listed cell alone cannot see it");
        assert!((c.ground_height(15.0, 1.0).unwrap() - 5.0).abs() < 1e-4);
        assert!((c.ground_height(14.0, 1.0).unwrap() - 5.0).abs() < 1e-4); // on the cell border
        let h = c.raycast([15.0, -10.0, 1.0], [0.0, 1.0, 0.0], 100.0).unwrap();
        assert!((h.t - 15.0).abs() < 1e-3);
        let h = c.raycast([35.0, -5.0, 2.0], [-1.0, 0.5, 0.0], 100.0).unwrap();
        assert!((h.point[1] - 5.0).abs() < 1e-3 && (h.point[0] - 15.0).abs() < 1e-3);
    }

    #[test]
    fn rejects_bad_buffers() {
        assert_eq!(Collision::parse(&[0u8; 16]).err(), Some(CollisionError::NoCollision));
        let mut b = synthetic();
        let n = b.len();
        b.truncate(n - 400); // grid cut short
        assert!(Collision::parse(&b).is_err());
        let mut b = synthetic();
        b[4] = 2; // placement count no longer matches
        assert!(matches!(Collision::parse(&b), Err(CollisionError::BadHeader(_))));
    }

    /// Real zone check (skipped without an install): collision ground height at random
    /// points lies on the visual mesh, and a downward ray agrees with `ground_height`.
    #[test]
    fn real_zone_ground_matches_visual_mesh() {
        use crate::crypt::{self, ZoneKeys};
        let Some(root) = crate::find_install_root() else { return };
        let Ok(keys) = ZoneKeys::from_install(&root) else { return };
        let Ok(data) = std::fs::read(root.join("ROM").join("0").join("101.DAT")) else { return };
        let (mut mzb, mut meshes) = (None, HashMap::new());
        for c in crate::chunks(&data).filter_map(Result::ok) {
            let mut p = c.data.to_vec();
            if c.kind == crypt::MZB_KIND && mzb.is_none() {
                crypt::decode_mzb(&mut p, &keys);
                mzb = Some(p);
            } else if c.kind == crypt::MMB_KIND {
                crypt::decode_mmb(&mut p, &keys);
                if let Ok(m) = crate::mmb::parse(&p) {
                    meshes.insert(m.name.clone(), m);
                }
            }
        }
        let mzb = mzb.expect("zone has an MZB");
        let col = Collision::parse(&mzb).expect("collision parses");
        // visual triangles in world space (same transform order as the zone renderer)
        let rot = |v: [f32; 3], r: [f32; 3]| {
            let (sx, cx) = r[0].sin_cos();
            let (sy, cy) = r[1].sin_cos();
            let (sz, cz) = r[2].sin_cos();
            let v = [v[0], v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx];
            let v = [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy];
            [v[0] * cz - v[1] * sz, v[0] * sz + v[1] * cz, v[2]]
        };
        let mut vis: Vec<[[f32; 3]; 3]> = Vec::new();
        for pl in crypt::placements(&mzb) {
            let Some(m) = meshes.get(&pl.mesh) else { continue };
            for sm in &m.submeshes {
                let w: Vec<[f32; 3]> = sm
                    .vertices
                    .iter()
                    .map(|v| {
                        let sc = [v.position[0] * pl.scale[0], v.position[1] * pl.scale[1], v.position[2] * pl.scale[2]];
                        let r = rot(sc, pl.rotation);
                        [r[0] + pl.position[0], r[1] + pl.position[1], r[2] + pl.position[2]]
                    })
                    .collect();
                for t in sm.indices.chunks_exact(3) {
                    vis.push([w[t[0] as usize], w[t[1] as usize], w[t[2] as usize]]);
                }
            }
        }
        let cells: Vec<usize> = col.non_empty_cells().map(|(c, _)| c).collect();
        let mut seed = 7u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f32 / (1u64 << 31) as f32
        };
        let (mut n, mut on_visual) = (0, 0);
        for _ in 0..200 {
            let c = cells[(rnd() * cells.len() as f32) as usize % cells.len()];
            let x = col.origin[0] + ((c % col.cols) as f32 + rnd()) * CELL;
            let z = col.origin[1] + ((c / col.cols) as f32 + rnd()) * CELL;
            let Some(h) = col.ground_height(x, z) else { continue };
            n += 1;
            let ray = col.raycast([x, h - 50.0, z], [0.0, 1.0, 0.0], 200.0).expect("ray hits");
            assert!((ray.point[1] - h).abs() < 0.05, "ray {} vs height {h}", ray.point[1]);
            if vis.iter().any(|w| height_in_tri(w, x, z).is_some_and(|y| (y - h).abs() < 0.25)) {
                on_visual += 1;
            }
        }
        assert!(n > 150, "only {n} of 200 samples had collision");
        assert!(on_visual * 100 >= n * 95, "{on_visual}/{n} collision heights sit on the visual mesh");
    }

    /// Real zone check (skipped without an install): the hairline at x 141, z -8.00..-7.97 in
    /// zone 101 (a mesh listed in the cell below z = -8 spills 0.1 yalm into the cell above) is
    /// closed, and fine random lines over its walkable area have no short holes.
    #[test]
    fn real_zone_has_no_hairline_gaps() {
        use crate::crypt::{self, ZoneKeys};
        let Some(root) = crate::find_install_root() else { return };
        let Ok(keys) = ZoneKeys::from_install(&root) else { return };
        let Ok(data) = std::fs::read(root.join("ROM").join("0").join("101.DAT")) else { return };
        let Some(mut p) = crate::chunks(&data).filter_map(Result::ok).find(|c| c.kind == crypt::MZB_KIND).map(|c| c.data.to_vec()) else { return };
        crypt::decode_mzb(&mut p, &keys);
        let col = Collision::parse(&p).expect("collision parses");
        let mut miss = Vec::new();
        for i in 0..=100 {
            let z = -8.1 + i as f32 * 0.002;
            if col.ground_height(141.0, z).is_none() {
                miss.push(z);
            }
            let d = 141.0 + (i as f32 - 50.0) * 0.001;
            if col.ground_height(d, -7.985).is_none() {
                miss.push(d);
            }
        }
        assert!(miss.is_empty(), "no surface at {miss:?}");
        assert!(col.ground_height_listed_only(141.0, -7.985).is_none(), "the old own-cell lookup had the gap");
        // random lines from cells with collision, one sample per 0.01 yalm; a hole of <= 0.15 yalm
        // with surface on both sides is a gap
        let ne: Vec<usize> = col.non_empty_cells().map(|(c, _)| c).collect();
        let mut seed = 5u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f32 / (1u64 << 31) as f32
        };
        let (mut samples, mut gaps) = (0u64, 0u64);
        for _ in 0..40 {
            let c = ne[(rnd() * ne.len() as f32) as usize % ne.len()];
            let x0 = col.origin[0] + ((c % col.cols) as f32 + rnd()) * CELL;
            let z0 = col.origin[1] + ((c / col.cols) as f32 + rnd()) * CELL;
            let ang = rnd() * std::f32::consts::TAU;
            let (mut run, mut seen) = (0u64, false);
            for i in 0..2000 {
                let (x, z) = (x0 + ang.cos() * 0.01 * i as f32, z0 + ang.sin() * 0.01 * i as f32);
                if col.ground_height(x, z).is_some() {
                    if seen {
                        samples += run + 1;
                        if run <= 15 {
                            gaps += run;
                        }
                    }
                    seen = true;
                    run = 0;
                } else {
                    run += 1;
                }
            }
        }
        assert!(samples > 10_000);
        assert!(gaps * 10_000 <= samples, "{gaps} gap samples of {samples}");
        // rays agree with heights across the old gap
        let h = col.ground_height(141.0, -7.985).unwrap();
        let r = col.raycast([141.0, h - 20.0, -7.985], [0.0, 1.0, 0.0], 100.0).expect("ray hits at the old gap");
        assert!((r.point[1] - h).abs() < 0.05);
    }
}
