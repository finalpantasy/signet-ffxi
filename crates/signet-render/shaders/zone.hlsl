// Original Signet zone shader: textured, two-sided directional lighting, three alpha modes.
//   PSOpaque: texture alpha ignored.
//   PSCutout: alpha test at 0.5 (mips keep coverage, see mips.rs), no blending.
//   PSBlend:  texture alpha times vertex alpha (PS2 range, 0x80 = opaque), alpha blended.
#pragma pack_matrix(row_major)
cbuffer Frame : register(b0)
{
    float4x4 view_proj;
    float4 light;   // xyz: direction toward the light, w: 1 = modulate by vertex color
    float4 fog_col; // rgb: fog color, w: fog start distance
    float4 amb;     // rgb: ambient light, w: fog end distance (<= 0: no fog)
    float4 dif;     // rgb: diffuse light, w: 1 = zone environment lighting, 0 = old neutral shading
    float4 sky0;    // rgb: sky color at the horizon, w: horizon height in NDC y
    float4 sky1;    // rgb: sky color 1/3 of the way to the zenith
    float4 sky2;    // rgb: 2/3
    float4 sky3;    // rgb: zenith
    float4 eye;     // xyz: camera position, w: light gain
};

// Overall gain on ambient + diffuse*N.L is eye.w (env.rs: 0.62 outdoors, 1.0 for indoor keys; a tuning guess).
Texture2D tex : register(t0);
SamplerState smp : register(s0);

struct VIn { float3 p : POSITION; float3 n : NORMAL; float2 uv : TEXCOORD0; float4 c : COLOR0; };
struct VOut { float4 pos : SV_Position; float3 n : NORMAL; float2 uv : TEXCOORD0; float4 c : COLOR0; float3 wp : TEXCOORD1; };

VOut VSMain(VIn i)
{
    VOut o;
    o.pos = mul(view_proj, float4(i.p, 1.0));
    o.n = i.n; o.uv = i.uv; o.c = i.c; o.wp = i.p;
    return o;
}

// Skinned actors: bone palette (row-major 3x4 per bone, 256 bones max) in b1. Positions are
// local to each bone and already scaled by the weight for dual-bone vertices, so the bone
// translation is scaled by the weight here.
cbuffer Bones : register(b1) { float4 bone[768]; };
struct SIn
{
    float3 p0 : POSITION0; float3 n0 : NORMAL0; float3 p1 : POSITION1; float3 n1 : NORMAL1;
    float2 w : BLENDWEIGHT; uint2 b : BLENDINDICES; float2 uv : TEXCOORD0;
};
float3 bone_rot(uint b, float3 v)
{
    return float3(dot(bone[b * 3].xyz, v), dot(bone[b * 3 + 1].xyz, v), dot(bone[b * 3 + 2].xyz, v));
}
float3 bone_pos(uint b)
{
    return float3(bone[b * 3].w, bone[b * 3 + 1].w, bone[b * 3 + 2].w);
}
VOut VSSkin(SIn i)
{
    VOut o;
    float3 p = bone_rot(i.b.x, i.p0) + bone_pos(i.b.x) * i.w.x + bone_rot(i.b.y, i.p1) + bone_pos(i.b.y) * i.w.y;
    o.pos = mul(view_proj, float4(p, 1.0));
    o.wp = p;
    o.n = bone_rot(i.b.x, i.n0) + bone_rot(i.b.y, i.n1);
    o.uv = i.uv;
    o.c = float4(1, 1, 1, 1);
    return o;
}

float3 lit(VOut i, float3 albedo)
{
    float3 n = normalize(i.n);
    float nl = abs(dot(n, light.xyz));
    float3 shade = (dif.w > 0.5) ? saturate((amb.rgb + dif.rgb * nl) * eye.w) : float3(1, 1, 1) * (0.40 + 0.60 * nl);
    float3 c = albedo * shade;
    if (light.w > 0.5) c *= saturate(i.c.rgb * 2.0);
    if (amb.w > 0.0)
    {
        float f = saturate((distance(i.wp, eye.xyz) - fog_col.w) / max(amb.w - fog_col.w, 1.0));
        c = lerp(c, fog_col.rgb, f);
    }
    return c;
}

float4 PSOpaque(VOut i) : SV_Target
{
    return float4(lit(i, tex.Sample(smp, i.uv).rgb), 1.0);
}

float4 PSCutout(VOut i) : SV_Target
{
    float4 t = tex.Sample(smp, i.uv);
    clip(t.a - 0.5);
    return float4(lit(i, t.rgb), 1.0);
}

float4 PSBlend(VOut i) : SV_Target
{
    float4 t = tex.Sample(smp, i.uv);
    float a = t.a * saturate(i.c.a * 2.0);
    clip(a - 0.01);
    return float4(lit(i, t.rgb), a);
}

// Sky: a full-screen triangle drawn first (no depth). The gradient runs from the horizon line
// up the screen; everything below the horizon is the horizon color.
struct SkyOut { float4 pos : SV_Position; float y : TEXCOORD0; };
SkyOut VSSky(uint id : SV_VertexID)
{
    float2 p = float2((id << 1) & 2, id & 2);
    SkyOut o;
    o.pos = float4(p * 2.0 - 1.0, 0.0, 1.0);
    o.y = p.y * 2.0 - 1.0;
    return o;
}
float4 PSSky(SkyOut i) : SV_Target
{
    float s = saturate((i.y - sky0.w) * 0.5) * 3.0;
    float3 c = (s < 1.0) ? lerp(sky0.rgb, sky1.rgb, s) : (s < 2.0) ? lerp(sky1.rgb, sky2.rgb, s - 1.0) : lerp(sky2.rgb, sky3.rgb, s - 2.0);
    return float4(c, 1.0);
}
