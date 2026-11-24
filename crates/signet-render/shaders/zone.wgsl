// Vulkan port of zone.hlsl (naga turns this into SPIR-V at build time). Same math, entry
// points and resource layout; keep the two files in step. Matrices arrive row-major from the
// CPU, so the uniform holds the transpose and `v * m` is HLSL's `mul(M, v)`.
struct Frame {
    view_proj: mat4x4<f32>,
    light: vec4<f32>, // xyz: direction toward the light, w: 1 = modulate by vertex color
};
struct Bones {
    bone: array<vec4<f32>, 768>,
};
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<uniform> bones: Bones;
@group(0) @binding(2) var smp: sampler;
@group(1) @binding(0) var tex: texture_2d<f32>;

struct VIn {
    @location(0) p: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) c: vec4<f32>,
};
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) n: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) c: vec4<f32>,
};

@vertex
fn VSMain(i: VIn) -> VOut {
    var o: VOut;
    o.pos = vec4<f32>(i.p, 1.0) * frame.view_proj;
    o.n = i.n;
    o.uv = i.uv;
    o.c = i.c;
    return o;
}

struct SIn {
    @location(0) p0: vec3<f32>,
    @location(1) n0: vec3<f32>,
    @location(2) p1: vec3<f32>,
    @location(3) n1: vec3<f32>,
    @location(4) w: vec2<f32>,
    @location(5) b: vec2<u32>,
    @location(6) uv: vec2<f32>,
};

fn bone_rot(b: u32, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(bones.bone[b * 3u].xyz, v),
        dot(bones.bone[b * 3u + 1u].xyz, v),
        dot(bones.bone[b * 3u + 2u].xyz, v),
    );
}
fn bone_pos(b: u32) -> vec3<f32> {
    return vec3<f32>(bones.bone[b * 3u].w, bones.bone[b * 3u + 1u].w, bones.bone[b * 3u + 2u].w);
}

@vertex
fn VSSkin(i: SIn) -> VOut {
    var o: VOut;
    let p = bone_rot(i.b.x, i.p0) + bone_pos(i.b.x) * i.w.x + bone_rot(i.b.y, i.p1) + bone_pos(i.b.y) * i.w.y;
    o.pos = vec4<f32>(p, 1.0) * frame.view_proj;
    o.n = bone_rot(i.b.x, i.n0) + bone_rot(i.b.y, i.n1);
    o.uv = i.uv;
    o.c = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    return o;
}

fn lit(i: VOut, albedo: vec3<f32>) -> vec3<f32> {
    let n = normalize(i.n);
    let shade = 0.40 + 0.60 * abs(dot(n, frame.light.xyz));
    var c = albedo * shade;
    if (frame.light.w > 0.5) {
        c = c * clamp(i.c.rgb * 2.0, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return c;
}

@fragment
fn PSOpaque(i: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(lit(i, textureSample(tex, smp, i.uv).rgb), 1.0);
}

@fragment
fn PSCutout(i: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, smp, i.uv);
    if (t.a - 0.5 < 0.0) {
        discard;
    }
    return vec4<f32>(lit(i, t.rgb), 1.0);
}

@fragment
fn PSBlend(i: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, smp, i.uv);
    let a = t.a * clamp(i.c.a * 2.0, 0.0, 1.0);
    if (a - 0.01 < 0.0) {
        discard;
    }
    return vec4<f32>(lit(i, t.rgb), a);
}
