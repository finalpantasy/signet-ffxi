//! Direct3D 12 backend: device, pipeline, uploads, one frame of zone drawing.
use std::mem::ManuallyDrop;

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        Graphics::{
            Direct3D::{
                D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob,
                Fxc::{D3DCOMPILE_ENABLE_STRICTNESS, D3DCompile},
            },
            Direct3D12::*,
            Dxgi::{Common::*, *},
        },
        System::Threading::{CreateEventW, WaitForSingleObject},
    },
    core::{Interface, PCSTR},
};

use crate::actor::{ActorModel, ActorVertex, Bone3x4};
use crate::backend::{self, Backend, FrameParams, MAX_ACTOR_INSTANCES, PALETTE_BYTES, bytes_of, pass_draws};
use crate::env::EnvParams;
pub use crate::backend::{Res, SKY};
use crate::scene::{Draw, Pass, Scene, Vertex};

const SHADER: &[u8] = include_bytes!("../shaders/zone.hlsl");
pub const COLOR_FORMAT: DXGI_FORMAT = DXGI_FORMAT_R8G8B8A8_UNORM;
const DEPTH_FORMAT: DXGI_FORMAT = DXGI_FORMAT_D32_FLOAT;

/// Root constants for the frame cbuffer in zone.hlsl, in 32-bit values.
const FRAME_DWORDS: u32 = 52;

/// Height of the horizon line on screen (NDC y), from far points in several horizontal
/// directions; a roll-free camera puts them all on one line.
fn horizon_ndc_y(vp: &[f32; 16], eye: [f32; 3]) -> f32 {
    let (mut sum, mut n) = (0.0, 0);
    for k in 0..16 {
        let a = k as f32 * std::f32::consts::TAU / 16.0;
        let p = [eye[0] + a.cos() * 1.0e5, eye[1], eye[2] + a.sin() * 1.0e5, 1.0];
        let row = |r: usize| (0..4).map(|c| vp[r * 4 + c] * p[c]).sum::<f32>();
        let (y, w) = (row(1), row(3));
        if w > 1.0 {
            sum += y / w;
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum / n as f32 }
}

/// The frame cbuffer as root constants: view-proj, light, then the zone environment (or the old
/// fixed look when `env` is `None`).
fn frame_constants_d3d(f: &FrameParams, vertex_colors: bool, env: Option<&EnvParams>) -> [f32; FRAME_DWORDS as usize] {
    let mut c = [0f32; FRAME_DWORDS as usize];
    c[..16].copy_from_slice(&f.view_proj);
    c[16..19].copy_from_slice(&f.light);
    c[19] = if vertex_colors { 1.0 } else { 0.0 };
    if let Some(e) = env {
        c[20..23].copy_from_slice(&e.fog_color);
        c[23] = e.fog_start.max(0.0);
        c[24..27].copy_from_slice(&e.ambient);
        c[27] = if e.fog_end > 0.0 { e.fog_end.max(e.fog_start + 1.0) } else { 0.0 };
        c[28..31].copy_from_slice(&e.diffuse);
        c[31] = 1.0;
        for i in 0..4 {
            c[32 + 4 * i..35 + 4 * i].copy_from_slice(&e.sky[i]);
        }
        c[35] = horizon_ndc_y(&f.view_proj, f.eye);
    }
    c[48..51].copy_from_slice(&f.eye);
    c[51] = env.map_or(1.0, |e| e.gain);
    c
}
fn api<T>(stage: &str, r: windows::core::Result<T>) -> Res<T> {
    r.map_err(|e| format!("{stage}: {e}"))
}

fn heap(kind: D3D12_HEAP_TYPE) -> D3D12_HEAP_PROPERTIES {
    D3D12_HEAP_PROPERTIES { Type: kind, CreationNodeMask: 1, VisibleNodeMask: 1, ..Default::default() }
}

fn buffer_desc(size: u64) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Width: size.max(1),
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        ..Default::default()
    }
}

fn tex_desc(w: u32, h: u32, mips: u16, format: DXGI_FORMAT, flags: D3D12_RESOURCE_FLAGS) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Width: w as u64,
        Height: h,
        DepthOrArraySize: 1,
        MipLevels: mips,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: flags,
        ..Default::default()
    }
}

fn create_resource(
    device: &ID3D12Device,
    kind: D3D12_HEAP_TYPE,
    desc: &D3D12_RESOURCE_DESC,
    state: D3D12_RESOURCE_STATES,
    clear: Option<&D3D12_CLEAR_VALUE>,
) -> Res<ID3D12Resource> {
    let mut r: Option<ID3D12Resource> = None;
    api("CreateCommittedResource", unsafe {
        device.CreateCommittedResource(&heap(kind), D3D12_HEAP_FLAG_NONE, desc, state, clear.map(|c| c as *const _), &mut r)
    })?;
    r.ok_or_else(|| "no resource".to_string())
}

pub fn barrier(list: &ID3D12GraphicsCommandList, res: &ID3D12Resource, from: D3D12_RESOURCE_STATES, to: D3D12_RESOURCE_STATES) {
    if from == to {
        return;
    }
    let mut b = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: ManuallyDrop::new(Some(res.clone())),
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: from,
                StateAfter: to,
            }),
        },
    };
    unsafe {
        list.ResourceBarrier(std::slice::from_ref(&b));
        let t = &mut *b.Anonymous.Transition;
        ManuallyDrop::drop(&mut t.pResource);
        ManuallyDrop::drop(&mut b.Anonymous.Transition);
    }
}

fn copy_loc_sub(res: &ID3D12Resource, sub: u32) -> D3D12_TEXTURE_COPY_LOCATION {
    D3D12_TEXTURE_COPY_LOCATION {
        pResource: ManuallyDrop::new(Some(res.clone())),
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: sub },
    }
}
fn copy_loc_placed(res: &ID3D12Resource, fp: D3D12_PLACED_SUBRESOURCE_FOOTPRINT) -> D3D12_TEXTURE_COPY_LOCATION {
    D3D12_TEXTURE_COPY_LOCATION {
        pResource: ManuallyDrop::new(Some(res.clone())),
        Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: fp },
    }
}
fn drop_loc(mut l: D3D12_TEXTURE_COPY_LOCATION) {
    unsafe { ManuallyDrop::drop(&mut l.pResource) }
}

fn blob_text(b: &ID3DBlob) -> String {
    unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(b.GetBufferPointer().cast::<u8>(), b.GetBufferSize())).into_owned() }
}

fn compile(entry: &std::ffi::CStr, target: &std::ffi::CStr) -> Res<ID3DBlob> {
    unsafe {
        let (mut code, mut msgs) = (None, None);
        let r = D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            PCSTR(c"zone.hlsl".as_ptr().cast()),
            None,
            None,
            PCSTR(entry.as_ptr().cast()),
            PCSTR(target.as_ptr().cast()),
            D3DCOMPILE_ENABLE_STRICTNESS,
            0,
            &mut code,
            Some(&mut msgs),
        );
        if let Err(e) = r {
            let detail = msgs.as_ref().map(blob_text).unwrap_or_default();
            return Err(format!("HLSL {entry:?}: {e}: {detail}"));
        }
        code.ok_or_else(|| "no bytecode".to_string())
    }
}

/// One model's buffers and draws (a mob, a race-and-equipment combination, the player).
struct ActorSet {
    _vb: ID3D12Resource,
    _ib: ID3D12Resource,
    vbv: D3D12_VERTEX_BUFFER_VIEW,
    ibv: D3D12_INDEX_BUFFER_VIEW,
    /// (SRV slot, first index, index count)
    draws: Vec<(usize, u32, u32)>,
}

struct ActorGpu {
    sets: Vec<ActorSet>,
    palette: ID3D12Resource,
    palette_ptr: *mut u8,
    /// Palette i is drawn with set `inst[i]`; its length is how many copies the next frames draw.
    inst: Vec<usize>,
}

/// SRV descriptors kept free after the zone textures for actor textures added while running.
const EXTRA_SRV: u32 = 4096;

pub struct Gpu {
    pub device: ID3D12Device,
    pub queue: ID3D12CommandQueue,
    pub adapter_name: String,
    fence: ID3D12Fence,
    event: HANDLE,
    fence_value: u64,
    alloc: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    root: ID3D12RootSignature,
    /// Opaque, cutout, blend (indexed by `Pass`).
    psos: [ID3D12PipelineState; 3],
    skin_pso: ID3D12PipelineState,
    actor: Option<ActorGpu>,
    srv_heap: ID3D12DescriptorHeap,
    srv_inc: u32,
    rtv_heap: ID3D12DescriptorHeap,
    rtv_inc: u32,
    dsv_heap: ID3D12DescriptorHeap,
    depth: Option<(ID3D12Resource, u32, u32)>,
    _textures: Vec<ID3D12Resource>,
    /// Next free SRV slot for actor textures.
    next_srv: u32,
    srv_capacity: u32,
    actor_textures: Vec<ID3D12Resource>,
    vb: ID3D12Resource,
    ib: ID3D12Resource,
    vbv: D3D12_VERTEX_BUFFER_VIEW,
    ibv: D3D12_INDEX_BUFFER_VIEW,
    draws: Vec<Draw>,
    pub vertex_colors: bool,
    sky_pso: ID3D12PipelineState,
    /// Zone environment; `None` keeps the old flat sky, no fog and neutral shading.
    pub env: Option<EnvParams>,
    offscreen: Option<Offscreen>,
}

struct Offscreen {
    target: ID3D12Resource,
    rb: ID3D12Resource,
    fp: D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
    w: u32,
    h: u32,
}

impl Drop for Gpu {
    fn drop(&mut self) {
        let _ = self.wait_idle();
        unsafe {
            let _ = CloseHandle(self.event);
        }
    }
}

impl Gpu {
    pub fn new(scene: &Scene) -> Res<Gpu> {
        unsafe {
            let factory: IDXGIFactory4 = api("CreateDXGIFactory2", CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)))?;
            let mut picked = None;
            for i in 0..16 {
                let a = match factory.EnumAdapters1(i) {
                    Ok(a) => a,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(e) => return Err(format!("EnumAdapters1: {e}")),
                };
                let d = api("GetDesc1", a.GetDesc1())?;
                if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                    continue;
                }
                let mut dev: Option<ID3D12Device> = None;
                if D3D12CreateDevice(&a, D3D_FEATURE_LEVEL_11_0, &mut dev).is_ok() {
                    let n = d.Description.iter().position(|c| *c == 0).unwrap_or(d.Description.len());
                    picked = Some((dev.ok_or("no device")?, String::from_utf16_lossy(&d.Description[..n])));
                    break;
                }
            }
            let (device, adapter_name) = picked.ok_or("no hardware D3D12 adapter")?;
            let queue: ID3D12CommandQueue = api(
                "CreateCommandQueue",
                device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC { Type: D3D12_COMMAND_LIST_TYPE_DIRECT, ..Default::default() }),
            )?;
            let fence: ID3D12Fence = api("CreateFence", device.CreateFence(0, D3D12_FENCE_FLAG_NONE))?;
            let event = api("CreateEvent", CreateEventW(None, false, false, None))?;
            let alloc: ID3D12CommandAllocator =
                api("CreateCommandAllocator", device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT))?;
            let list: ID3D12GraphicsCommandList =
                api("CreateCommandList", device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &alloc, None))?;
            api("Close", list.Close())?;

            // Root signature: FRAME_DWORDS root constants (see `frame_constants_d3d`) and one SRV table.
            let range = D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: 1,
                BaseShaderRegister: 0,
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
            };
            let params = [
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        Constants: D3D12_ROOT_CONSTANTS { ShaderRegister: 0, RegisterSpace: 0, Num32BitValues: FRAME_DWORDS },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                },
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE { NumDescriptorRanges: 1, pDescriptorRanges: &range },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                },
                // Bone palette for skinned actors (root CBV, b1).
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_CBV,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        Descriptor: D3D12_ROOT_DESCRIPTOR { ShaderRegister: 1, RegisterSpace: 0 },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_VERTEX,
                },
            ];
            let sampler = D3D12_STATIC_SAMPLER_DESC {
                Filter: D3D12_FILTER_ANISOTROPIC,
                AddressU: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                AddressV: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                AddressW: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                MipLODBias: 0.0,
                MaxAnisotropy: 16,
                ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
                BorderColor: D3D12_STATIC_BORDER_COLOR_OPAQUE_BLACK,
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
                ShaderRegister: 0,
                RegisterSpace: 0,
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            };
            let rs_desc = D3D12_ROOT_SIGNATURE_DESC {
                NumParameters: 3,
                pParameters: params.as_ptr(),
                NumStaticSamplers: 1,
                pStaticSamplers: &sampler,
                Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
            };
            let (mut blob, mut err) = (None, None);
            if let Err(e) = D3D12SerializeRootSignature(&rs_desc, D3D_ROOT_SIGNATURE_VERSION_1, &mut blob, Some(&mut err)) {
                let d = err.as_ref().map(blob_text).unwrap_or_default();
                return Err(format!("SerializeRootSignature: {e}: {d}"));
            }
            let blob: ID3DBlob = blob.ok_or("no root blob")?;
            let root: ID3D12RootSignature = api(
                "CreateRootSignature",
                device.CreateRootSignature(0, std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())),
            )?;

            let vs = compile(c"VSMain", c"vs_5_0")?;
            let vs_skin = compile(c"VSSkin", c"vs_5_0")?;
            let el = |name: PCSTR, fmt: DXGI_FORMAT, off: u32| D3D12_INPUT_ELEMENT_DESC {
                SemanticName: name,
                SemanticIndex: 0,
                Format: fmt,
                InputSlot: 0,
                AlignedByteOffset: off,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            };
            let elements = [
                el(PCSTR(c"POSITION".as_ptr().cast()), DXGI_FORMAT_R32G32B32_FLOAT, 0),
                el(PCSTR(c"NORMAL".as_ptr().cast()), DXGI_FORMAT_R32G32B32_FLOAT, 12),
                el(PCSTR(c"TEXCOORD".as_ptr().cast()), DXGI_FORMAT_R32G32_FLOAT, 24),
                el(PCSTR(c"COLOR".as_ptr().cast()), DXGI_FORMAT_B8G8R8A8_UNORM, 32),
            ];
            let sel = |name: &[u8], idx: u32, fmt: DXGI_FORMAT, off: u32| D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(name.as_ptr()),
                SemanticIndex: idx,
                Format: fmt,
                InputSlot: 0,
                AlignedByteOffset: off,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            };
            // Matches `ActorVertex`: p0, n0, p1, n1, weights, bone indices, uv.
            let skin_elements = [
                sel(b"POSITION\0", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0),
                sel(b"NORMAL\0", 0, DXGI_FORMAT_R32G32B32_FLOAT, 12),
                sel(b"POSITION\0", 1, DXGI_FORMAT_R32G32B32_FLOAT, 24),
                sel(b"NORMAL\0", 1, DXGI_FORMAT_R32G32B32_FLOAT, 36),
                sel(b"BLENDWEIGHT\0", 0, DXGI_FORMAT_R32G32_FLOAT, 48),
                sel(b"BLENDINDICES\0", 0, DXGI_FORMAT_R32G32_UINT, 56),
                sel(b"TEXCOORD\0", 0, DXGI_FORMAT_R32G32_FLOAT, 64),
            ];
            let stencil = D3D12_DEPTH_STENCILOP_DESC {
                StencilFailOp: D3D12_STENCIL_OP_KEEP,
                StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
                StencilPassOp: D3D12_STENCIL_OP_KEEP,
                StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            };
            let make_pso = |entry: &std::ffi::CStr, blend_on: bool, depth_on: bool, vs: &ID3DBlob, elements: &[D3D12_INPUT_ELEMENT_DESC]| -> Res<ID3D12PipelineState> {
                let ps = compile(entry, c"ps_5_0")?;
                let blend = D3D12_RENDER_TARGET_BLEND_DESC {
                    BlendEnable: blend_on.into(),
                    SrcBlend: if blend_on { D3D12_BLEND_SRC_ALPHA } else { D3D12_BLEND_ONE },
                    DestBlend: if blend_on { D3D12_BLEND_INV_SRC_ALPHA } else { D3D12_BLEND_ZERO },
                    BlendOp: D3D12_BLEND_OP_ADD,
                    SrcBlendAlpha: D3D12_BLEND_ONE,
                    DestBlendAlpha: D3D12_BLEND_ZERO,
                    BlendOpAlpha: D3D12_BLEND_OP_ADD,
                    LogicOp: D3D12_LOGIC_OP_NOOP,
                    RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
                    ..Default::default()
                };
                let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
                    pRootSignature: ManuallyDrop::new(Some(root.clone())),
                    VS: D3D12_SHADER_BYTECODE { pShaderBytecode: vs.GetBufferPointer(), BytecodeLength: vs.GetBufferSize() },
                    PS: D3D12_SHADER_BYTECODE { pShaderBytecode: ps.GetBufferPointer(), BytecodeLength: ps.GetBufferSize() },
                    BlendState: D3D12_BLEND_DESC { RenderTarget: [blend; 8], ..Default::default() },
                    SampleMask: u32::MAX,
                    RasterizerState: D3D12_RASTERIZER_DESC {
                        FillMode: D3D12_FILL_MODE_SOLID,
                        CullMode: D3D12_CULL_MODE_NONE,
                        DepthClipEnable: true.into(),
                        ..Default::default()
                    },
                    DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
                        DepthEnable: depth_on.into(),
                        // Blended draws test depth but do not write it.
                        DepthWriteMask: if blend_on { D3D12_DEPTH_WRITE_MASK_ZERO } else { D3D12_DEPTH_WRITE_MASK_ALL },
                        DepthFunc: D3D12_COMPARISON_FUNC_LESS,
                        StencilEnable: false.into(),
                        StencilReadMask: 0xff,
                        StencilWriteMask: 0xff,
                        FrontFace: stencil,
                        BackFace: stencil,
                    },
                    InputLayout: D3D12_INPUT_LAYOUT_DESC { pInputElementDescs: elements.as_ptr(), NumElements: elements.len() as u32 },
                    PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
                    NumRenderTargets: 1,
                    DSVFormat: DEPTH_FORMAT,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    ..Default::default()
                };
                desc.RTVFormats[0] = COLOR_FORMAT;
                let r = device.CreateGraphicsPipelineState::<ID3D12PipelineState>(&desc);
                ManuallyDrop::drop(&mut desc.pRootSignature);
                api("CreateGraphicsPipelineState", r)
            };
            let psos = [
                make_pso(c"PSOpaque", false, true, &vs, &elements)?,
                make_pso(c"PSCutout", false, true, &vs, &elements)?,
                make_pso(c"PSBlend", true, true, &vs, &elements)?,
            ];
            let skin_pso = make_pso(c"PSCutout", false, true, &vs_skin, &skin_elements)?;
            let vs_sky = compile(c"VSSky", c"vs_5_0")?;
            let sky_pso = make_pso(c"PSSky", false, false, &vs_sky, &[])?;

            let srv_heap: ID3D12DescriptorHeap = api(
                "SRV heap",
                device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                    NumDescriptors: scene.slots.len() as u32 + 1 + EXTRA_SRV,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                    NodeMask: 0,
                }),
            )?;
            let rtv_heap: ID3D12DescriptorHeap = api(
                "RTV heap",
                device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: 3,
                    ..Default::default()
                }),
            )?;
            let dsv_heap: ID3D12DescriptorHeap = api(
                "DSV heap",
                device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
                    NumDescriptors: 1,
                    ..Default::default()
                }),
            )?;
            let srv_inc = device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
            let rtv_inc = device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);

            // Placeholder buffers; replaced by upload_scene below.
            let placeholder = create_resource(&device, D3D12_HEAP_TYPE_DEFAULT, &buffer_desc(4), D3D12_RESOURCE_STATE_COMMON, None)?;
            let mut gpu = Gpu {
                device,
                queue,
                adapter_name,
                fence,
                event,
                fence_value: 0,
                alloc,
                list,
                root,
                psos,
                skin_pso,
                sky_pso,
                env: None,
                actor: None,
                srv_heap,
                srv_inc,
                rtv_heap,
                rtv_inc,
                dsv_heap,
                depth: None,
                _textures: Vec::new(),
                next_srv: scene.slots.len() as u32 + 1,
                srv_capacity: scene.slots.len() as u32 + 1 + EXTRA_SRV,
                actor_textures: Vec::new(),
                vb: placeholder.clone(),
                ib: placeholder,
                vbv: Default::default(),
                ibv: Default::default(),
                draws: Vec::new(),
                vertex_colors: std::env::var_os("SIGNET_VCOL").is_some(),
                offscreen: None,
            };
            gpu.upload_scene(scene)?;
            Ok(gpu)
        }
    }

    /// Sets the zone environment used by every later frame (`None`: the old flat look).
    pub fn set_environment(&mut self, env: Option<EnvParams>) {
        self.env = env;
    }

    pub fn wait_idle(&mut self) -> Res<()> {
        unsafe {
            self.fence_value += 1;
            api("Signal", self.queue.Signal(&self.fence, self.fence_value))?;
            if self.fence.GetCompletedValue() < self.fence_value {
                api("SetEventOnCompletion", self.fence.SetEventOnCompletion(self.fence_value, self.event))?;
                if WaitForSingleObject(self.event, 20_000).0 != 0 {
                    return Err("GPU wait timed out".into());
                }
            }
        }
        Ok(())
    }

    fn submit(&mut self) -> Res<()> {
        unsafe {
            api("Close", self.list.Close())?;
            let l: ID3D12CommandList = api("cast", self.list.cast())?;
            self.queue.ExecuteCommandLists(&[Some(l)]);
        }
        self.wait_idle()
    }

    /// Records uploads of full mip chains into new textures and creates their SRVs at consecutive
    /// slots from `first_slot`. The caller owns the command list (reset before, submitted after) and
    /// keeps the returned upload buffer alive in `keep` until the GPU is done.
    unsafe fn record_textures(&self, chains: Vec<Vec<crate::mips::Level>>, first_slot: u32, keep: &mut Vec<ID3D12Resource>) -> Res<Vec<ID3D12Resource>> {
        unsafe {
            struct Plan {
                desc: D3D12_RESOURCE_DESC,
                layouts: Vec<D3D12_PLACED_SUBRESOURCE_FOOTPRINT>,
                rows: Vec<u32>,
                row_bytes: Vec<u64>,
                chain: Vec<crate::mips::Level>,
            }
            let mut plans = Vec::new();
            let mut total = 0u64;
            for chain in chains {
                let desc = tex_desc(chain[0].w, chain[0].h, chain.len() as u16, COLOR_FORMAT, D3D12_RESOURCE_FLAG_NONE);
                let n = chain.len();
                let mut layouts = vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); n];
                let (mut rows, mut row_bytes, mut size) = (vec![0u32; n], vec![0u64; n], 0u64);
                self.device.GetCopyableFootprints(
                    &desc,
                    0,
                    n as u32,
                    total,
                    Some(layouts.as_mut_ptr()),
                    Some(rows.as_mut_ptr()),
                    Some(row_bytes.as_mut_ptr()),
                    Some(&mut size),
                );
                total = (total + size + 511) & !511;
                plans.push(Plan { desc, layouts, rows, row_bytes, chain });
            }
            let mut textures = Vec::new();
            if plans.is_empty() {
                return Ok(textures);
            }
            let up = create_resource(&self.device, D3D12_HEAP_TYPE_UPLOAD, &buffer_desc(total), D3D12_RESOURCE_STATE_GENERIC_READ, None)?;
            let mut base = std::ptr::null_mut();
            api("Map", up.Map(0, None, Some(&mut base)))?;
            let base = base.cast::<u8>();
            let mut cpu = self.srv_heap.GetCPUDescriptorHandleForHeapStart();
            cpu.ptr += (first_slot * self.srv_inc) as usize;
            for plan in &plans {
                let tex = create_resource(&self.device, D3D12_HEAP_TYPE_DEFAULT, &plan.desc, D3D12_RESOURCE_STATE_COPY_DEST, None)?;
                for (i, level) in plan.chain.iter().enumerate() {
                    let (data, w) = (&level.rgba, &level.w);
                    let fp = plan.layouts[i];
                    let src_pitch = *w as usize * 4;
                    for r in 0..plan.rows[i] as usize {
                        let dst = base.add(fp.Offset as usize + r * fp.Footprint.RowPitch as usize);
                        std::ptr::copy_nonoverlapping(data.as_ptr().add(r * src_pitch), dst, plan.row_bytes[i] as usize);
                    }
                    let (dl, sl) = (copy_loc_sub(&tex, i as u32), copy_loc_placed(&up, fp));
                    self.list.CopyTextureRegion(&dl, 0, 0, 0, &sl, None);
                    drop_loc(dl);
                    drop_loc(sl);
                }
                barrier(&self.list, &tex, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE);
                self.device.CreateShaderResourceView(&tex, None, cpu);
                cpu.ptr += self.srv_inc as usize;
                textures.push(tex);
            }
            up.Unmap(0, None);
            keep.push(up);
            Ok(textures)
        }
    }

    fn upload_scene(&mut self, scene: &Scene) -> Res<()> {
        unsafe {
            api("alloc reset", self.alloc.Reset())?;
            api("list reset", self.list.Reset(&self.alloc, None))?;
            let mut keep: Vec<ID3D12Resource> = Vec::new();

            // Geometry: upload heap -> default heap.
            let vb_bytes = bytes_of(&scene.vertices);
            let ib_bytes = bytes_of(&scene.indices);
            let make = |bytes: &[u8], keep: &mut Vec<ID3D12Resource>| -> Res<ID3D12Resource> {
                let up = create_resource(&self.device, D3D12_HEAP_TYPE_UPLOAD, &buffer_desc(bytes.len() as u64), D3D12_RESOURCE_STATE_GENERIC_READ, None)?;
                let mut p = std::ptr::null_mut();
                api("Map", up.Map(0, None, Some(&mut p)))?;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), bytes.len());
                up.Unmap(0, None);
                let dst = create_resource(&self.device, D3D12_HEAP_TYPE_DEFAULT, &buffer_desc(bytes.len() as u64), D3D12_RESOURCE_STATE_COPY_DEST, None)?;
                self.list.CopyBufferRegion(&dst, 0, &up, 0, bytes.len() as u64);
                barrier(&self.list, &dst, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_GENERIC_READ);
                keep.push(up);
                Ok(dst)
            };
            self.vb = make(vb_bytes, &mut keep)?;
            self.ib = make(ib_bytes, &mut keep)?;
            self.vbv = D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: self.vb.GetGPUVirtualAddress(),
                SizeInBytes: vb_bytes.len() as u32,
                StrideInBytes: std::mem::size_of::<Vertex>() as u32,
            };
            self.ibv = D3D12_INDEX_BUFFER_VIEW {
                BufferLocation: self.ib.GetGPUVirtualAddress(),
                SizeInBytes: ib_bytes.len() as u32,
                Format: DXGI_FORMAT_R32_UINT,
            };
            self.draws = scene.draws.clone();

            let textures = self.record_textures(backend::texture_chains(scene), 0, &mut keep)?;
            self._textures = textures;
            self.submit()?;
            drop(keep);
        }
        Ok(())
    }

    /// Uploads a model's vertex and index buffers as a new actor set. `tex_slots[i]` is the SRV
    /// slot of `model.images[i]`.
    fn make_actor_set(&mut self, model: &ActorModel, tex_slots: &[usize]) -> Res<ActorSet> {
        if model.skeleton.bones.len() + 1 > 256 {
            return Err("skeleton has more than 255 bones".into());
        }
        unsafe {
            api("alloc reset", self.alloc.Reset())?;
            api("list reset", self.list.Reset(&self.alloc, None))?;
            let mut keep: Vec<ID3D12Resource> = Vec::new();
            let vb_bytes = bytes_of(&model.vertices);
            let ib_bytes = bytes_of(&model.indices);
            let mut make = |bytes: &[u8]| -> Res<ID3D12Resource> {
                let up = create_resource(&self.device, D3D12_HEAP_TYPE_UPLOAD, &buffer_desc(bytes.len().max(16) as u64), D3D12_RESOURCE_STATE_GENERIC_READ, None)?;
                let mut p = std::ptr::null_mut();
                api("Map", up.Map(0, None, Some(&mut p)))?;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), bytes.len());
                up.Unmap(0, None);
                let dst = create_resource(&self.device, D3D12_HEAP_TYPE_DEFAULT, &buffer_desc(bytes.len().max(16) as u64), D3D12_RESOURCE_STATE_COPY_DEST, None)?;
                self.list.CopyBufferRegion(&dst, 0, &up, 0, bytes.len() as u64);
                barrier(&self.list, &dst, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_GENERIC_READ);
                keep.push(up);
                Ok(dst)
            };
            let vb = make(vb_bytes)?;
            let ib = make(ib_bytes)?;
            let vbv = D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: vb.GetGPUVirtualAddress(),
                SizeInBytes: vb_bytes.len() as u32,
                StrideInBytes: std::mem::size_of::<ActorVertex>() as u32,
            };
            let ibv = D3D12_INDEX_BUFFER_VIEW {
                BufferLocation: ib.GetGPUVirtualAddress(),
                SizeInBytes: ib_bytes.len() as u32,
                Format: DXGI_FORMAT_R32_UINT,
            };
            let draws = backend::actor_draws(model, tex_slots);
            self.submit()?;
            drop(keep);
            Ok(ActorSet { _vb: vb, _ib: ib, vbv, ibv, draws })
        }
    }

    fn ensure_palettes(&mut self) -> Res<()> {
        if self.actor.is_none() {
            unsafe {
                let palette = create_resource(&self.device, D3D12_HEAP_TYPE_UPLOAD, &buffer_desc(PALETTE_BYTES * MAX_ACTOR_INSTANCES as u64), D3D12_RESOURCE_STATE_GENERIC_READ, None)?;
                let mut pp = std::ptr::null_mut();
                api("Map palette", palette.Map(0, None, Some(&mut pp)))?;
                self.actor = Some(ActorGpu { sets: Vec::new(), palette, palette_ptr: pp.cast(), inst: Vec::new() });
            }
        }
        Ok(())
    }

    /// Uploads a skinned actor as set 0 (the player). `tex_slots[i]` is the SRV slot of
    /// `model.images[i]`. One copy is drawn until `set_actor_count` or `set_instances` say otherwise.
    pub fn set_actor(&mut self, model: &ActorModel, tex_slots: &[usize]) -> Res<()> {
        let set = self.make_actor_set(model, tex_slots)?;
        self.ensure_palettes()?;
        if let Some(a) = &mut self.actor {
            a.sets.clear();
            a.sets.push(set);
            a.inst = vec![0];
        }
        Ok(())
    }

    /// Adds a model whose textures (`model.images`) are uploaded now into free SRV slots; returns
    /// the set number for `set_instances`. Used for monsters and equipped characters met while
    /// playing.
    pub fn add_actor_set(&mut self, model: &ActorModel) -> Res<usize> {
        use crate::mips::{TexClass, classify};
        self.ensure_palettes()?;
        let n = model.images.len() as u32;
        if self.next_srv + n > self.srv_capacity {
            return Err("out of texture slots for actors".into());
        }
        let first = self.next_srv;
        let chains: Vec<Vec<crate::mips::Level>> = model
            .images
            .iter()
            .map(|img| {
                let class = if classify(&img.rgba) == TexClass::Opaque { TexClass::Opaque } else { TexClass::Cutout };
                crate::mips::chain(&img.rgba, img.width, img.height, class)
            })
            .collect();
        let tex = unsafe {
            api("alloc reset", self.alloc.Reset())?;
            api("list reset", self.list.Reset(&self.alloc, None))?;
            let mut keep = Vec::new();
            let t = self.record_textures(chains, first, &mut keep)?;
            self.submit()?;
            drop(keep);
            t
        };
        self.actor_textures.extend(tex);
        self.next_srv += n;
        let slots: Vec<usize> = (0..n as usize).map(|i| first as usize + i).collect();
        let set = self.make_actor_set(model, &slots)?;
        let a = self.actor.as_mut().ok_or("no actor palettes")?;
        a.sets.push(set);
        Ok(a.sets.len() - 1)
    }

    /// Which set each palette copy draws: copy `i` uses palette `i` and set `sets[i]`.
    pub fn set_instances(&mut self, sets: &[usize]) {
        if let Some(a) = &mut self.actor {
            let n = a.sets.len();
            a.inst = sets.iter().take(MAX_ACTOR_INSTANCES).map(|&s| s.min(n.saturating_sub(1))).collect();
        }
    }

    /// Writes the bone palette used by the next `render` (the previous frame has finished).
    pub fn set_palette(&mut self, pal: &[Bone3x4]) {
        if let Some(a) = &self.actor {
            let n = pal.len().min(256);
            unsafe { std::ptr::copy_nonoverlapping(pal.as_ptr().cast::<u8>(), a.palette_ptr, n * 48) };
        }
    }

    /// Writes the bone palette of copy `i` (0 is the one `set_palette` writes).
    pub fn set_palette_at(&mut self, i: usize, pal: &[Bone3x4]) {
        if let (Some(a), true) = (&self.actor, i < MAX_ACTOR_INSTANCES) {
            let n = pal.len().min(256);
            unsafe { std::ptr::copy_nonoverlapping(pal.as_ptr().cast::<u8>(), a.palette_ptr.add(i * PALETTE_BYTES as usize), n * 48) };
        }
    }

    fn rtv_handle(&self, slot: u32) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        let mut h = unsafe { self.rtv_heap.GetCPUDescriptorHandleForHeapStart() };
        h.ptr += (slot * self.rtv_inc) as usize;
        h
    }

    pub fn make_color_target(&self, w: u32, h: u32, state: D3D12_RESOURCE_STATES) -> Res<ID3D12Resource> {
        let clear = D3D12_CLEAR_VALUE { Format: COLOR_FORMAT, Anonymous: D3D12_CLEAR_VALUE_0 { Color: SKY } };
        create_resource(&self.device, D3D12_HEAP_TYPE_DEFAULT, &tex_desc(w, h, 1, COLOR_FORMAT, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET), state, Some(&clear))
    }

    pub fn set_rtv(&self, slot: u32, res: &ID3D12Resource) {
        unsafe { self.device.CreateRenderTargetView(res, None, self.rtv_handle(slot)) }
    }

    fn ensure_depth(&mut self, w: u32, h: u32) -> Res<()> {
        if matches!(&self.depth, Some((_, dw, dh)) if *dw == w && *dh == h) {
            return Ok(());
        }
        let clear = D3D12_CLEAR_VALUE {
            Format: DEPTH_FORMAT,
            Anonymous: D3D12_CLEAR_VALUE_0 { DepthStencil: D3D12_DEPTH_STENCIL_VALUE { Depth: 1.0, Stencil: 0 } },
        };
        let d = create_resource(
            &self.device,
            D3D12_HEAP_TYPE_DEFAULT,
            &tex_desc(w, h, 1, DEPTH_FORMAT, D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL),
            D3D12_RESOURCE_STATE_DEPTH_WRITE,
            Some(&clear),
        )?;
        unsafe { self.device.CreateDepthStencilView(&d, None, self.dsv_heap.GetCPUDescriptorHandleForHeapStart()) };
        self.depth = Some((d, w, h));
        Ok(())
    }

    /// Draws one frame into `target` (RTV slot `slot`), which rests in state `rest`.
    /// If `readback` is given, the frame is also copied into it.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        target: &ID3D12Resource,
        slot: u32,
        rest: D3D12_RESOURCE_STATES,
        w: u32,
        h: u32,
        view_proj: &[f32; 16],
        light: [f32; 3],
        eye: [f32; 3],
        readback: Option<(&ID3D12Resource, D3D12_PLACED_SUBRESOURCE_FOOTPRINT)>,
    ) -> Res<()> {
        self.ensure_depth(w, h)?;
        unsafe {
            api("alloc reset", self.alloc.Reset())?;
            api("list reset", self.list.Reset(&self.alloc, &self.psos[0]))?;
            let l = &self.list;
            barrier(l, target, rest, D3D12_RESOURCE_STATE_RENDER_TARGET);
            let rtv = self.rtv_handle(slot);
            let dsv = self.dsv_heap.GetCPUDescriptorHandleForHeapStart();
            l.OMSetRenderTargets(1, Some(&rtv), false, Some(&dsv));
            let clear = self.env.map_or(SKY, |e| e.clear());
            l.ClearRenderTargetView(rtv, &clear, None);
            l.ClearDepthStencilView(dsv, D3D12_CLEAR_FLAG_DEPTH, 1.0, 0, None);
            l.SetGraphicsRootSignature(&self.root);
            l.SetDescriptorHeaps(&[Some(self.srv_heap.clone())]);
            l.RSSetViewports(&[D3D12_VIEWPORT { Width: w as f32, Height: h as f32, MaxDepth: 1.0, ..Default::default() }]);
            l.RSSetScissorRects(&[windows::Win32::Foundation::RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 }]);
            l.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            l.IASetVertexBuffers(0, Some(&[self.vbv]));
            l.IASetIndexBuffer(Some(&self.ibv));
            let c = frame_constants_d3d(&FrameParams { view_proj: *view_proj, light, eye }, self.vertex_colors, self.env.as_ref());
            l.SetGraphicsRoot32BitConstants(0, FRAME_DWORDS, c.as_ptr().cast(), 0);
            if self.env.is_some() {
                l.SetPipelineState(&self.sky_pso);
                l.DrawInstanced(3, 1, 0, 0);
            }
            let start = self.srv_heap.GetGPUDescriptorHandleForHeapStart();
            let draw = |d: &Draw| {
                let mut g = start;
                g.ptr += d.texture as u64 * self.srv_inc as u64;
                l.SetGraphicsRootDescriptorTable(1, g);
                l.DrawIndexedInstanced(d.index_count, 1, d.first_index, 0, 0);
            };
            // Opaque, then alpha-tested, then blended back to front (depth write off).
            for pass in [Pass::Opaque, Pass::Cutout] {
                l.SetPipelineState(&self.psos[pass as usize]);
                pass_draws(&self.draws, pass, eye).into_iter().for_each(&draw);
            }
            if let Some(a) = &self.actor {
                l.SetPipelineState(&self.skin_pso);
                let mut bound = usize::MAX;
                for (inst, &set) in a.inst.iter().enumerate() {
                    let Some(st) = a.sets.get(set) else { continue };
                    if bound != set {
                        l.IASetVertexBuffers(0, Some(&[st.vbv]));
                        l.IASetIndexBuffer(Some(&st.ibv));
                        bound = set;
                    }
                    l.SetGraphicsRootConstantBufferView(2, a.palette.GetGPUVirtualAddress() + inst as u64 * PALETTE_BYTES);
                    for &(slot, first, count) in &st.draws {
                        let mut g = start;
                        g.ptr += slot as u64 * self.srv_inc as u64;
                        l.SetGraphicsRootDescriptorTable(1, g);
                        l.DrawIndexedInstanced(count, 1, first, 0, 0);
                    }
                }
                l.IASetVertexBuffers(0, Some(&[self.vbv]));
                l.IASetIndexBuffer(Some(&self.ibv));
            }
            l.SetPipelineState(&self.psos[Pass::Blend as usize]);
            pass_draws(&self.draws, Pass::Blend, eye).into_iter().for_each(draw);
            if let Some((rb, fp)) = readback {
                barrier(l, target, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_COPY_SOURCE);
                let (dl, sl) = (copy_loc_placed(rb, fp), copy_loc_sub(target, 0));
                l.CopyTextureRegion(&dl, 0, 0, 0, &sl, None);
                drop_loc(dl);
                drop_loc(sl);
                barrier(l, target, D3D12_RESOURCE_STATE_COPY_SOURCE, rest);
            } else {
                barrier(l, target, D3D12_RESOURCE_STATE_RENDER_TARGET, rest);
            }
        }
        self.submit()
    }

    /// Creates a readback buffer and its footprint for a `w` x `h` RGBA8 target.
    pub fn make_readback(&self, w: u32, h: u32) -> Res<(ID3D12Resource, D3D12_PLACED_SUBRESOURCE_FOOTPRINT)> {
        let desc = tex_desc(w, h, 1, COLOR_FORMAT, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET);
        let mut fp = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
        let mut size = 0u64;
        unsafe { self.device.GetCopyableFootprints(&desc, 0, 1, 0, Some(&mut fp), None, None, Some(&mut size)) };
        let rb = create_resource(&self.device, D3D12_HEAP_TYPE_READBACK, &buffer_desc(size), D3D12_RESOURCE_STATE_COPY_DEST, None)?;
        Ok((rb, fp))
    }

    pub fn read_rgba(rb: &ID3D12Resource, fp: &D3D12_PLACED_SUBRESOURCE_FOOTPRINT, w: u32, h: u32) -> Res<Vec<u8>> {
        let pitch = fp.Footprint.RowPitch as usize;
        let mut out = vec![0u8; (w * h * 4) as usize];
        unsafe {
            let mut p = std::ptr::null_mut();
            api("Map readback", rb.Map(0, None, Some(&mut p)))?;
            for y in 0..h as usize {
                std::ptr::copy_nonoverlapping(p.cast::<u8>().add(y * pitch), out.as_mut_ptr().add(y * w as usize * 4), w as usize * 4);
            }
            rb.Unmap(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }));
        }
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Ok(out)
    }
}

impl Backend for Gpu {
    fn adapter_name(&self) -> &str {
        &self.adapter_name
    }
    fn set_actor(&mut self, model: &ActorModel, tex_slots: &[usize]) -> Res<()> {
        Gpu::set_actor(self, model, tex_slots)
    }
    fn set_palette(&mut self, pal: &[Bone3x4]) {
        Gpu::set_palette(self, pal)
    }
    fn begin_offscreen(&mut self, w: u32, h: u32) -> Res<()> {
        let target = self.make_color_target(w, h, D3D12_RESOURCE_STATE_COPY_SOURCE)?;
        self.set_rtv(0, &target);
        let (rb, fp) = self.make_readback(w, h)?;
        self.offscreen = Some(Offscreen { target, rb, fp, w, h });
        Ok(())
    }
    fn render_offscreen(&mut self, f: &FrameParams) -> Res<()> {
        let o = self.offscreen.take().ok_or("begin_offscreen not called")?;
        let r = self.render(&o.target, 0, D3D12_RESOURCE_STATE_COPY_SOURCE, o.w, o.h, &f.view_proj, f.light, f.eye, None);
        self.offscreen = Some(o);
        r
    }
    fn render_readback(&mut self, f: &FrameParams) -> Res<Vec<u8>> {
        let o = self.offscreen.take().ok_or("begin_offscreen not called")?;
        let r = self
            .render(&o.target, 0, D3D12_RESOURCE_STATE_COPY_SOURCE, o.w, o.h, &f.view_proj, f.light, f.eye, Some((&o.rb, o.fp)))
            .and_then(|_| Gpu::read_rgba(&o.rb, &o.fp, o.w, o.h));
        self.offscreen = Some(o);
        r
    }
}
