//! Vulkan backend (ash): same passes, depth, samplers, mips and skinning as the D3D12 backend,
//! offscreen only. Shaders come from shaders/zone.wgsl (SPIR-V built by build.rs).
use std::ffi::{CStr, c_void};
use std::sync::atomic::{AtomicU32, Ordering};

use ash::{Device, Entry, Instance, vk};

use crate::actor::{ActorModel, ActorVertex, Bone3x4};
use crate::backend::{self, Backend, FrameParams, PALETTE_BYTES, Res, SKY, bytes_of, pass_draws};
use crate::scene::{Draw, Pass, Scene, Vertex};

const COLOR_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const DEPTH_FORMAT: vk::Format = vk::Format::D32_SFLOAT;
const FRAME_UBO_BYTES: u64 = 80;

macro_rules! spv {
    ($name:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/", $name, ".spv"))
    };
}

/// Validation messages seen (warnings and errors), printed as they arrive.
pub static VALIDATION_ERRORS: AtomicU32 = AtomicU32::new(0);
pub static VALIDATION_WARNINGS: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn debug_cb(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut c_void,
) -> vk::Bool32 {
    let msg = unsafe { data.as_ref().and_then(|d| d.message_as_c_str()).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default() };
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        VALIDATION_ERRORS.fetch_add(1, Ordering::Relaxed);
        eprintln!("[vk validation ERROR] {msg}");
    } else {
        VALIDATION_WARNINGS.fetch_add(1, Ordering::Relaxed);
        eprintln!("[vk validation warning] {msg}");
    }
    vk::FALSE
}

fn e<T>(stage: &str, r: Result<T, vk::Result>) -> Res<T> {
    r.map_err(|x| format!("{stage}: {x}"))
}

struct Buf {
    buf: vk::Buffer,
    mem: vk::DeviceMemory,
}

struct Img {
    img: vk::Image,
    mem: vk::DeviceMemory,
    view: vk::ImageView,
}

struct ActorGpu {
    vb: Buf,
    ib: Buf,
    draws: Vec<(usize, u32, u32)>,
}

struct Offscreen {
    w: u32,
    h: u32,
    color: Img,
    depth: Img,
    fb: vk::Framebuffer,
    readback: Buf,
    readback_ptr: *const u8,
}

pub struct VkGpu {
    _entry: Entry,
    instance: Instance,
    debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    pub validation: bool,
    pub adapter_name: String,
    mem_props: vk::PhysicalDeviceMemoryProperties,
    device: Device,
    queue: vk::Queue,
    pool: vk::CommandPool,
    cb: vk::CommandBuffer,
    fence: vk::Fence,
    sampler: vk::Sampler,
    set0_layout: vk::DescriptorSetLayout,
    set1_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    desc_pool: vk::DescriptorPool,
    set0: vk::DescriptorSet,
    tex_sets: Vec<vk::DescriptorSet>,
    render_pass: vk::RenderPass,
    modules: Vec<vk::ShaderModule>,
    /// Opaque, cutout, blend (indexed by `Pass`).
    pipelines: [vk::Pipeline; 3],
    skin_pipeline: vk::Pipeline,
    frame_ubo: Buf,
    frame_ptr: *mut u8,
    bones: Buf,
    bones_ptr: *mut u8,
    textures: Vec<Img>,
    vb: Buf,
    ib: Buf,
    draws: Vec<Draw>,
    actor: Option<ActorGpu>,
    offscreen: Option<Offscreen>,
    pub vertex_colors: bool,
}

impl VkGpu {
    pub fn new(scene: &Scene) -> Res<VkGpu> {
        unsafe {
            let entry = Entry::load().map_err(|x| format!("load vulkan-1.dll: {x}"))?;
            // Validation layers: debug builds only, and only if the Khronos layer is installed.
            let want_validation = cfg!(debug_assertions) && std::env::var_os("SIGNET_VK_VALIDATION").is_none_or(|v| v != "0")
                || std::env::var_os("SIGNET_VK_VALIDATION").is_some_and(|v| v == "1");
            let layer_name = c"VK_LAYER_KHRONOS_validation";
            let have_layer = e("layers", entry.enumerate_instance_layer_properties())?
                .iter()
                .any(|l| l.layer_name_as_c_str().is_ok_and(|n| n == layer_name));
            let have_debug_ext = e("extensions", entry.enumerate_instance_extension_properties(None))?
                .iter()
                .any(|x| x.extension_name_as_c_str().is_ok_and(|n| n == ash::ext::debug_utils::NAME));
            let validation = want_validation && have_layer && have_debug_ext;
            if want_validation && !validation {
                eprintln!("vk: validation requested but the Khronos validation layer is not installed (Vulkan SDK); running without it");
            }
            let layers: Vec<*const i8> = if validation { vec![layer_name.as_ptr()] } else { vec![] };
            let exts: Vec<*const i8> = if validation { vec![ash::ext::debug_utils::NAME.as_ptr()] } else { vec![] };
            let app = vk::ApplicationInfo::default().application_name(c"signet-render").api_version(vk::API_VERSION_1_1);
            let instance = e(
                "vkCreateInstance",
                entry.create_instance(&vk::InstanceCreateInfo::default().application_info(&app).enabled_layer_names(&layers).enabled_extension_names(&exts), None),
            )?;
            let debug = if validation {
                let du = ash::ext::debug_utils::Instance::new(&entry, &instance);
                let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                    .message_severity(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR)
                    .message_type(
                        vk::DebugUtilsMessageTypeFlagsEXT::GENERAL | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                    )
                    .pfn_user_callback(Some(debug_cb));
                let m = e("debug messenger", du.create_debug_utils_messenger(&info, None))?;
                Some((du, m))
            } else {
                None
            };

            // Physical device: first discrete GPU with a graphics queue, else any GPU with one.
            let mut best: Option<(u32, vk::PhysicalDevice, u32, vk::PhysicalDeviceProperties)> = None;
            for pd in e("enumerate devices", instance.enumerate_physical_devices())? {
                let props = instance.get_physical_device_properties(pd);
                let Some(q) = instance.get_physical_device_queue_family_properties(pd).iter().position(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS)) else {
                    continue;
                };
                let rank = match props.device_type {
                    vk::PhysicalDeviceType::DISCRETE_GPU => 0,
                    vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                    _ => 2,
                };
                if best.as_ref().is_none_or(|(r, ..)| rank < *r) {
                    best = Some((rank, pd, q as u32, props));
                }
            }
            let (_, pd, qfam, props) = best.ok_or("no Vulkan device with a graphics queue")?;
            let adapter_name = props.device_name_as_c_str().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
            let mem_props = instance.get_physical_device_memory_properties(pd);
            let features = vk::PhysicalDeviceFeatures::default().sampler_anisotropy(true);
            let qinfo = [vk::DeviceQueueCreateInfo::default().queue_family_index(qfam).queue_priorities(&[1.0])];
            let device = e("vkCreateDevice", instance.create_device(pd, &vk::DeviceCreateInfo::default().queue_create_infos(&qinfo).enabled_features(&features), None))?;
            let queue = device.get_device_queue(qfam, 0);
            let pool = e("command pool", device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(qfam).flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER), None))?;
            let cb = e(
                "command buffer",
                device.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1)),
            )?[0];
            let fence = e("fence", device.create_fence(&vk::FenceCreateInfo::default(), None))?;

            // 16x anisotropic trilinear, wrapping, full LOD range (matches the D3D12 static sampler).
            let sampler = e(
                "sampler",
                device.create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
                        .address_mode_u(vk::SamplerAddressMode::REPEAT)
                        .address_mode_v(vk::SamplerAddressMode::REPEAT)
                        .address_mode_w(vk::SamplerAddressMode::REPEAT)
                        .anisotropy_enable(true)
                        .max_anisotropy(16.0)
                        .min_lod(0.0)
                        .max_lod(vk::LOD_CLAMP_NONE)
                        .border_color(vk::BorderColor::FLOAT_OPAQUE_BLACK),
                    None,
                ),
            )?;
            let samplers = [sampler];
            let stages = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
            let set0_bindings = [
                vk::DescriptorSetLayoutBinding::default().binding(0).descriptor_type(vk::DescriptorType::UNIFORM_BUFFER).descriptor_count(1).stage_flags(stages),
                vk::DescriptorSetLayoutBinding::default().binding(1).descriptor_type(vk::DescriptorType::UNIFORM_BUFFER).descriptor_count(1).stage_flags(vk::ShaderStageFlags::VERTEX),
                vk::DescriptorSetLayoutBinding::default().binding(2).descriptor_type(vk::DescriptorType::SAMPLER).stage_flags(vk::ShaderStageFlags::FRAGMENT).immutable_samplers(&samplers),
            ];
            let set0_layout = e("set0 layout", device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&set0_bindings), None))?;
            let set1_bindings = [vk::DescriptorSetLayoutBinding::default().binding(0).descriptor_type(vk::DescriptorType::SAMPLED_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::FRAGMENT)];
            let set1_layout = e("set1 layout", device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&set1_bindings), None))?;
            let set_layouts = [set0_layout, set1_layout];
            let layout = e("pipeline layout", device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts), None))?;

            let ntex = scene.slots.len() + 1;
            let pool_sizes = [
                vk::DescriptorPoolSize { ty: vk::DescriptorType::UNIFORM_BUFFER, descriptor_count: 2 },
                vk::DescriptorPoolSize { ty: vk::DescriptorType::SAMPLER, descriptor_count: 1 },
                vk::DescriptorPoolSize { ty: vk::DescriptorType::SAMPLED_IMAGE, descriptor_count: ntex as u32 },
            ];
            let desc_pool = e(
                "descriptor pool",
                device.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(ntex as u32 + 1).pool_sizes(&pool_sizes), None),
            )?;
            let set0 = e("alloc set0", device.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(desc_pool).set_layouts(&[set0_layout])))?[0];
            let tex_layouts = vec![set1_layout; ntex];
            let tex_sets = e("alloc tex sets", device.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(desc_pool).set_layouts(&tex_layouts)))?;

            // Render pass: clear color + depth; color ends in TRANSFER_SRC for the readback copy.
            let attachments = [
                vk::AttachmentDescription::default()
                    .format(COLOR_FORMAT)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
                vk::AttachmentDescription::default()
                    .format(DEPTH_FORMAT)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
            ];
            let color_ref = [vk::AttachmentReference { attachment: 0, layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL }];
            let depth_ref = vk::AttachmentReference { attachment: 1, layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL };
            let subpasses = [vk::SubpassDescription::default().pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS).color_attachments(&color_ref).depth_stencil_attachment(&depth_ref)];
            let deps = [
                vk::SubpassDependency::default()
                    .src_subpass(vk::SUBPASS_EXTERNAL)
                    .dst_subpass(0)
                    .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS)
                    .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS)
                    .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE),
                vk::SubpassDependency::default()
                    .src_subpass(0)
                    .dst_subpass(vk::SUBPASS_EXTERNAL)
                    .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                    .dst_stage_mask(vk::PipelineStageFlags::TRANSFER)
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ),
            ];
            let render_pass = e("render pass", device.create_render_pass(&vk::RenderPassCreateInfo::default().attachments(&attachments).subpasses(&subpasses).dependencies(&deps), None))?;

            // Shaders and pipelines.
            let mut modules = Vec::new();
            let mut module = |bytes: &[u8]| -> Res<vk::ShaderModule> {
                let words: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                let m = e("shader module", device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None))?;
                modules.push(m);
                Ok(m)
            };
            let vs = module(spv!("VSMain"))?;
            let vs_skin = module(spv!("VSSkin"))?;
            let ps = [module(spv!("PSOpaque"))?, module(spv!("PSCutout"))?, module(spv!("PSBlend"))?];

            let vb_binding = |stride: usize| [vk::VertexInputBindingDescription { binding: 0, stride: stride as u32, input_rate: vk::VertexInputRate::VERTEX }];
            let attr = |location: u32, format: vk::Format, offset: u32| vk::VertexInputAttributeDescription { location, binding: 0, format, offset };
            let zone_bindings = vb_binding(std::mem::size_of::<Vertex>());
            let zone_attrs = [
                attr(0, vk::Format::R32G32B32_SFLOAT, 0),
                attr(1, vk::Format::R32G32B32_SFLOAT, 12),
                attr(2, vk::Format::R32G32_SFLOAT, 24),
                attr(3, vk::Format::B8G8R8A8_UNORM, 32),
            ];
            let skin_bindings = vb_binding(std::mem::size_of::<ActorVertex>());
            let skin_attrs = [
                attr(0, vk::Format::R32G32B32_SFLOAT, 0),
                attr(1, vk::Format::R32G32B32_SFLOAT, 12),
                attr(2, vk::Format::R32G32B32_SFLOAT, 24),
                attr(3, vk::Format::R32G32B32_SFLOAT, 36),
                attr(4, vk::Format::R32G32_SFLOAT, 48),
                attr(5, vk::Format::R32G32_UINT, 56),
                attr(6, vk::Format::R32G32_SFLOAT, 64),
            ];
            let make = |vs: vk::ShaderModule, vs_name: &CStr, ps: vk::ShaderModule, ps_name: &CStr, blend_on: bool, bindings: &[vk::VertexInputBindingDescription], attrs: &[vk::VertexInputAttributeDescription]| -> Res<vk::Pipeline> {
                let stages = [
                    vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vs).name(vs_name),
                    vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(ps).name(ps_name),
                ];
                let vi = vk::PipelineVertexInputStateCreateInfo::default().vertex_binding_descriptions(bindings).vertex_attribute_descriptions(attrs);
                let ia = vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_LIST);
                let vp = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
                let rs = vk::PipelineRasterizationStateCreateInfo::default().polygon_mode(vk::PolygonMode::FILL).cull_mode(vk::CullModeFlags::NONE).front_face(vk::FrontFace::COUNTER_CLOCKWISE).line_width(1.0);
                let ms = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
                // Blended draws test depth but do not write it.
                let ds = vk::PipelineDepthStencilStateCreateInfo::default().depth_test_enable(true).depth_write_enable(!blend_on).depth_compare_op(vk::CompareOp::LESS);
                let att = [vk::PipelineColorBlendAttachmentState::default()
                    .blend_enable(blend_on)
                    .src_color_blend_factor(if blend_on { vk::BlendFactor::SRC_ALPHA } else { vk::BlendFactor::ONE })
                    .dst_color_blend_factor(if blend_on { vk::BlendFactor::ONE_MINUS_SRC_ALPHA } else { vk::BlendFactor::ZERO })
                    .color_blend_op(vk::BlendOp::ADD)
                    .src_alpha_blend_factor(vk::BlendFactor::ONE)
                    .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
                    .alpha_blend_op(vk::BlendOp::ADD)
                    .color_write_mask(vk::ColorComponentFlags::RGBA)];
                let cbs = vk::PipelineColorBlendStateCreateInfo::default().attachments(&att);
                let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
                let dy = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);
                let info = vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&vi)
                    .input_assembly_state(&ia)
                    .viewport_state(&vp)
                    .rasterization_state(&rs)
                    .multisample_state(&ms)
                    .depth_stencil_state(&ds)
                    .color_blend_state(&cbs)
                    .dynamic_state(&dy)
                    .layout(layout)
                    .render_pass(render_pass)
                    .subpass(0);
                match device.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None) {
                    Ok(p) => Ok(p[0]),
                    Err((_, r)) => Err(format!("create pipeline: {r}")),
                }
            };
            let pipelines = [
                make(vs, c"VSMain", ps[0], c"PSOpaque", false, &zone_bindings, &zone_attrs)?,
                make(vs, c"VSMain", ps[1], c"PSCutout", false, &zone_bindings, &zone_attrs)?,
                make(vs, c"VSMain", ps[2], c"PSBlend", true, &zone_bindings, &zone_attrs)?,
            ];
            let skin_pipeline = make(vs_skin, c"VSSkin", ps[1], c"PSCutout", false, &skin_bindings, &skin_attrs)?;

            let mut gpu = VkGpu {
                _entry: entry,
                instance,
                debug,
                validation,
                adapter_name,
                mem_props,
                device,
                queue,
                pool,
                cb,
                fence,
                sampler,
                set0_layout,
                set1_layout,
                layout,
                desc_pool,
                set0,
                tex_sets,
                render_pass,
                modules,
                pipelines,
                skin_pipeline,
                frame_ubo: Buf { buf: vk::Buffer::null(), mem: vk::DeviceMemory::null() },
                frame_ptr: std::ptr::null_mut(),
                bones: Buf { buf: vk::Buffer::null(), mem: vk::DeviceMemory::null() },
                bones_ptr: std::ptr::null_mut(),
                textures: Vec::new(),
                vb: Buf { buf: vk::Buffer::null(), mem: vk::DeviceMemory::null() },
                ib: Buf { buf: vk::Buffer::null(), mem: vk::DeviceMemory::null() },
                draws: Vec::new(),
                actor: None,
                offscreen: None,
                vertex_colors: std::env::var_os("SIGNET_VCOL").is_some(),
            };
            // On error `gpu` drops and frees everything created so far.
            gpu.init_uniforms()?;
            gpu.upload_scene(scene)?;
            Ok(gpu)
        }
    }

    fn mem_type(&self, bits: u32, want: vk::MemoryPropertyFlags) -> Res<u32> {
        (0..self.mem_props.memory_type_count)
            .find(|&i| bits & (1 << i) != 0 && self.mem_props.memory_types[i as usize].property_flags.contains(want))
            .ok_or_else(|| format!("no memory type for {want:?}"))
    }

    fn buffer(&self, size: u64, usage: vk::BufferUsageFlags, props: vk::MemoryPropertyFlags) -> Res<Buf> {
        unsafe {
            let buf = e("create buffer", self.device.create_buffer(&vk::BufferCreateInfo::default().size(size.max(4)).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE), None))?;
            let req = self.device.get_buffer_memory_requirements(buf);
            let mem = e(
                "alloc buffer memory",
                self.device.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(self.mem_type(req.memory_type_bits, props)?), None),
            )?;
            e("bind buffer", self.device.bind_buffer_memory(buf, mem, 0))?;
            Ok(Buf { buf, mem })
        }
    }

    fn host_buffer(&self, size: u64, usage: vk::BufferUsageFlags) -> Res<(Buf, *mut u8)> {
        let b = self.buffer(size, usage, vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT)?;
        let p = e("map", unsafe { self.device.map_memory(b.mem, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty()) })?;
        Ok((b, p.cast()))
    }

    fn image(&self, w: u32, h: u32, mips: u32, format: vk::Format, usage: vk::ImageUsageFlags, aspect: vk::ImageAspectFlags) -> Res<Img> {
        unsafe {
            let img = e(
                "create image",
                self.device.create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(format)
                        .extent(vk::Extent3D { width: w, height: h, depth: 1 })
                        .mip_levels(mips)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(usage)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE)
                        .initial_layout(vk::ImageLayout::UNDEFINED),
                    None,
                ),
            )?;
            let req = self.device.get_image_memory_requirements(img);
            let mem = e(
                "alloc image memory",
                self.device.allocate_memory(
                    &vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(self.mem_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)?),
                    None,
                ),
            )?;
            e("bind image", self.device.bind_image_memory(img, mem, 0))?;
            let view = e(
                "image view",
                self.device.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(img)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: aspect, base_mip_level: 0, level_count: mips, base_array_layer: 0, layer_count: 1 }),
                    None,
                ),
            )?;
            Ok(Img { img, mem, view })
        }
    }

    /// Records into the shared command buffer, submits and waits.
    fn one_time(&self, record: impl FnOnce(vk::CommandBuffer)) -> Res<()> {
        unsafe {
            e("begin", self.device.begin_command_buffer(self.cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)))?;
            record(self.cb);
            e("end", self.device.end_command_buffer(self.cb))?;
            self.submit_wait()
        }
    }

    fn submit_wait(&self) -> Res<()> {
        unsafe {
            let cbs = [self.cb];
            e("submit", self.device.queue_submit(self.queue, &[vk::SubmitInfo::default().command_buffers(&cbs)], self.fence))?;
            e("wait", self.device.wait_for_fences(&[self.fence], true, 20_000_000_000))?;
            e("reset fence", self.device.reset_fences(&[self.fence]))
        }
    }

    /// Device-local buffer filled through a staging buffer.
    fn device_buffer(&self, bytes: &[u8], usage: vk::BufferUsageFlags) -> Res<Buf> {
        let (stage, p) = self.host_buffer(bytes.len() as u64, vk::BufferUsageFlags::TRANSFER_SRC)?;
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) };
        let dst = self.buffer(bytes.len() as u64, usage | vk::BufferUsageFlags::TRANSFER_DST, vk::MemoryPropertyFlags::DEVICE_LOCAL)?;
        let r = self.one_time(|cb| unsafe {
            self.device.cmd_copy_buffer(cb, stage.buf, dst.buf, &[vk::BufferCopy { src_offset: 0, dst_offset: 0, size: bytes.len() as u64 }]);
        });
        self.free_buf(&stage);
        r?;
        Ok(dst)
    }

    fn free_buf(&self, b: &Buf) {
        unsafe {
            self.device.destroy_buffer(b.buf, None);
            self.device.free_memory(b.mem, None);
        }
    }

    fn free_img(&self, i: &Img) {
        unsafe {
            self.device.destroy_image_view(i.view, None);
            self.device.destroy_image(i.img, None);
            self.device.free_memory(i.mem, None);
        }
    }

    fn init_uniforms(&mut self) -> Res<()> {
        let (f, fp) = self.host_buffer(FRAME_UBO_BYTES, vk::BufferUsageFlags::UNIFORM_BUFFER)?;
        self.frame_ubo = f;
        self.frame_ptr = fp;
        let (b, bp) = self.host_buffer(PALETTE_BYTES, vk::BufferUsageFlags::UNIFORM_BUFFER)?;
        unsafe { std::ptr::write_bytes(bp, 0, PALETTE_BYTES as usize) };
        self.bones = b;
        self.bones_ptr = bp;
        let fi = [vk::DescriptorBufferInfo { buffer: self.frame_ubo.buf, offset: 0, range: FRAME_UBO_BYTES }];
        let bi = [vk::DescriptorBufferInfo { buffer: self.bones.buf, offset: 0, range: PALETTE_BYTES }];
        let writes = [
            vk::WriteDescriptorSet::default().dst_set(self.set0).dst_binding(0).descriptor_type(vk::DescriptorType::UNIFORM_BUFFER).buffer_info(&fi),
            vk::WriteDescriptorSet::default().dst_set(self.set0).dst_binding(1).descriptor_type(vk::DescriptorType::UNIFORM_BUFFER).buffer_info(&bi),
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }

    fn upload_scene(&mut self, scene: &Scene) -> Res<()> {
        self.vb = self.device_buffer(bytes_of(&scene.vertices), vk::BufferUsageFlags::VERTEX_BUFFER)?;
        self.ib = self.device_buffer(bytes_of(&scene.indices), vk::BufferUsageFlags::INDEX_BUFFER)?;
        self.draws = scene.draws.clone();

        // Textures: CPU mip chains, one staging buffer, one submit.
        let chains = backend::texture_chains(scene);
        let total: usize = chains.iter().map(|c| c.iter().map(|l| l.rgba.len()).sum::<usize>()).sum();
        let (stage, sp) = self.host_buffer(total as u64, vk::BufferUsageFlags::TRANSFER_SRC)?;
        let mut off = 0usize;
        let mut copies: Vec<Vec<vk::BufferImageCopy>> = Vec::new();
        for chain in &chains {
            let mut v = Vec::new();
            for (i, l) in chain.iter().enumerate() {
                unsafe { std::ptr::copy_nonoverlapping(l.rgba.as_ptr(), sp.add(off), l.rgba.len()) };
                v.push(vk::BufferImageCopy {
                    buffer_offset: off as u64,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: i as u32, base_array_layer: 0, layer_count: 1 },
                    image_offset: vk::Offset3D::default(),
                    image_extent: vk::Extent3D { width: l.w, height: l.h, depth: 1 },
                });
                off += l.rgba.len();
            }
            copies.push(v);
        }
        let mut textures = Vec::new();
        for chain in &chains {
            textures.push(self.image(chain[0].w, chain[0].h, chain.len() as u32, COLOR_FORMAT, vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST, vk::ImageAspectFlags::COLOR)?);
        }
        let barrier = |img: vk::Image, mips: u32, from: vk::ImageLayout, to: vk::ImageLayout, sa: vk::AccessFlags, da: vk::AccessFlags| {
            vk::ImageMemoryBarrier::default()
                .old_layout(from)
                .new_layout(to)
                .src_access_mask(sa)
                .dst_access_mask(da)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(img)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: mips, base_array_layer: 0, layer_count: 1 })
        };
        let r = self.one_time(|cb| unsafe {
            for (t, (chain, c)) in textures.iter().zip(chains.iter().zip(&copies)) {
                let n = chain.len() as u32;
                self.device.cmd_pipeline_barrier(
                    cb,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier(t.img, n, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_WRITE)],
                );
                self.device.cmd_copy_buffer_to_image(cb, stage.buf, t.img, vk::ImageLayout::TRANSFER_DST_OPTIMAL, c);
                self.device.cmd_pipeline_barrier(
                    cb,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier(t.img, n, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::SHADER_READ)],
                );
            }
        });
        self.free_buf(&stage);
        let infos: Vec<[vk::DescriptorImageInfo; 1]> = textures
            .iter()
            .map(|t| [vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: t.view, image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL }])
            .collect();
        let writes: Vec<vk::WriteDescriptorSet> = infos
            .iter()
            .zip(&self.tex_sets)
            .map(|(i, &s)| vk::WriteDescriptorSet::default().dst_set(s).dst_binding(0).descriptor_type(vk::DescriptorType::SAMPLED_IMAGE).image_info(i))
            .collect();
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        self.textures = textures;
        r
    }

    fn free_offscreen(&mut self) {
        if let Some(o) = self.offscreen.take() {
            unsafe {
                self.device.destroy_framebuffer(o.fb, None);
                self.device.unmap_memory(o.readback.mem);
            }
            self.free_img(&o.color);
            self.free_img(&o.depth);
            self.free_buf(&o.readback);
        }
    }

    /// Records and submits one frame into the offscreen target; optionally copies it to the readback buffer.
    fn draw_frame(&mut self, f: &FrameParams, readback: bool) -> Res<()> {
        let o = self.offscreen.as_ref().ok_or("begin_offscreen not called")?;
        let (w, h) = (o.w, o.h);
        let consts = backend::frame_constants(f, self.vertex_colors);
        unsafe {
            std::ptr::copy_nonoverlapping(consts.as_ptr().cast::<u8>(), self.frame_ptr, FRAME_UBO_BYTES as usize);
            let d = &self.device;
            let cb = self.cb;
            e("begin", d.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)))?;
            let clears = [
                vk::ClearValue { color: vk::ClearColorValue { float32: SKY } },
                vk::ClearValue { depth_stencil: vk::ClearDepthStencilValue { depth: 1.0, stencil: 0 } },
            ];
            d.cmd_begin_render_pass(
                cb,
                &vk::RenderPassBeginInfo::default().render_pass(self.render_pass).framebuffer(o.fb).render_area(vk::Rect2D { offset: vk::Offset2D::default(), extent: vk::Extent2D { width: w, height: h } }).clear_values(&clears),
                vk::SubpassContents::INLINE,
            );
            // Negative height flips Y so D3D-style clip space (+y up) lands the right way up.
            d.cmd_set_viewport(cb, 0, &[vk::Viewport { x: 0.0, y: h as f32, width: w as f32, height: -(h as f32), min_depth: 0.0, max_depth: 1.0 }]);
            d.cmd_set_scissor(cb, 0, &[vk::Rect2D { offset: vk::Offset2D::default(), extent: vk::Extent2D { width: w, height: h } }]);
            d.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::GRAPHICS, self.layout, 0, &[self.set0], &[]);
            d.cmd_bind_vertex_buffers(cb, 0, &[self.vb.buf], &[0]);
            d.cmd_bind_index_buffer(cb, self.ib.buf, 0, vk::IndexType::UINT32);
            let draw = |dr: &Draw| {
                d.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::GRAPHICS, self.layout, 1, &[self.tex_sets[dr.texture]], &[]);
                d.cmd_draw_indexed(cb, dr.index_count, 1, dr.first_index, 0, 0);
            };
            // Opaque, then alpha-tested, then the actor, then blended back to front (depth write off).
            for pass in [Pass::Opaque, Pass::Cutout] {
                d.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, self.pipelines[pass as usize]);
                pass_draws(&self.draws, pass, f.eye).into_iter().for_each(&draw);
            }
            if let Some(a) = &self.actor {
                d.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, self.skin_pipeline);
                d.cmd_bind_vertex_buffers(cb, 0, &[a.vb.buf], &[0]);
                d.cmd_bind_index_buffer(cb, a.ib.buf, 0, vk::IndexType::UINT32);
                for &(slot, first, count) in &a.draws {
                    d.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::GRAPHICS, self.layout, 1, &[self.tex_sets[slot]], &[]);
                    d.cmd_draw_indexed(cb, count, 1, first, 0, 0);
                }
                d.cmd_bind_vertex_buffers(cb, 0, &[self.vb.buf], &[0]);
                d.cmd_bind_index_buffer(cb, self.ib.buf, 0, vk::IndexType::UINT32);
            }
            d.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, self.pipelines[Pass::Blend as usize]);
            pass_draws(&self.draws, Pass::Blend, f.eye).into_iter().for_each(&draw);
            d.cmd_end_render_pass(cb);
            if readback {
                d.cmd_copy_image_to_buffer(
                    cb,
                    o.color.img,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    o.readback.buf,
                    &[vk::BufferImageCopy {
                        buffer_offset: 0,
                        buffer_row_length: 0,
                        buffer_image_height: 0,
                        image_subresource: vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 },
                        image_offset: vk::Offset3D::default(),
                        image_extent: vk::Extent3D { width: w, height: h, depth: 1 },
                    }],
                );
                // Make the transfer write visible to the host read.
                d.cmd_pipeline_barrier(
                    cb,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::HOST,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default().src_access_mask(vk::AccessFlags::TRANSFER_WRITE).dst_access_mask(vk::AccessFlags::HOST_READ)],
                    &[],
                    &[],
                );
            }
            e("end", d.end_command_buffer(cb))?;
        }
        self.submit_wait()
    }
}

impl Drop for VkGpu {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
        }
        self.free_offscreen();
        if let Some(a) = self.actor.take() {
            self.free_buf(&a.vb);
            self.free_buf(&a.ib);
        }
        for t in std::mem::take(&mut self.textures) {
            self.free_img(&t);
        }
        for b in [&self.vb, &self.ib, &self.frame_ubo, &self.bones] {
            if b.buf != vk::Buffer::null() {
                self.free_buf(b);
            }
        }
        unsafe {
            let d = &self.device;
            for p in self.pipelines {
                d.destroy_pipeline(p, None);
            }
            d.destroy_pipeline(self.skin_pipeline, None);
            for &m in &self.modules {
                d.destroy_shader_module(m, None);
            }
            d.destroy_render_pass(self.render_pass, None);
            d.destroy_descriptor_pool(self.desc_pool, None);
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_descriptor_set_layout(self.set0_layout, None);
            d.destroy_descriptor_set_layout(self.set1_layout, None);
            d.destroy_sampler(self.sampler, None);
            d.destroy_fence(self.fence, None);
            d.destroy_command_pool(self.pool, None);
            d.destroy_device(None);
            if let Some((du, m)) = self.debug.take() {
                du.destroy_debug_utils_messenger(m, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

impl Backend for VkGpu {
    fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    fn set_actor(&mut self, model: &ActorModel, tex_slots: &[usize]) -> Res<()> {
        if model.skeleton.bones.len() + 1 > 256 {
            return Err("skeleton has more than 255 bones".into());
        }
        let vb = self.device_buffer(bytes_of(&model.vertices), vk::BufferUsageFlags::VERTEX_BUFFER)?;
        let ib = self.device_buffer(bytes_of(&model.indices), vk::BufferUsageFlags::INDEX_BUFFER)?;
        self.actor = Some(ActorGpu { vb, ib, draws: backend::actor_draws(model, tex_slots) });
        Ok(())
    }

    fn set_palette(&mut self, pal: &[Bone3x4]) {
        if self.actor.is_some() {
            let n = pal.len().min(256);
            unsafe { std::ptr::copy_nonoverlapping(pal.as_ptr().cast::<u8>(), self.bones_ptr, n * 48) };
        }
    }

    fn begin_offscreen(&mut self, w: u32, h: u32) -> Res<()> {
        self.free_offscreen();
        let color = self.image(w, h, 1, COLOR_FORMAT, vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC, vk::ImageAspectFlags::COLOR)?;
        let depth = self.image(w, h, 1, DEPTH_FORMAT, vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT, vk::ImageAspectFlags::DEPTH)?;
        let views = [color.view, depth.view];
        let fb = e(
            "framebuffer",
            unsafe { self.device.create_framebuffer(&vk::FramebufferCreateInfo::default().render_pass(self.render_pass).attachments(&views).width(w).height(h).layers(1), None) },
        )?;
        let (readback, rp) = self.host_buffer(w as u64 * h as u64 * 4, vk::BufferUsageFlags::TRANSFER_DST)?;
        self.offscreen = Some(Offscreen { w, h, color, depth, fb, readback, readback_ptr: rp });
        Ok(())
    }

    fn render_offscreen(&mut self, f: &FrameParams) -> Res<()> {
        self.draw_frame(f, false)
    }

    fn render_readback(&mut self, f: &FrameParams) -> Res<Vec<u8>> {
        self.draw_frame(f, true)?;
        let o = self.offscreen.as_ref().ok_or("begin_offscreen not called")?;
        let mut out = unsafe { std::slice::from_raw_parts(o.readback_ptr, (o.w * o.h * 4) as usize) }.to_vec();
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Ok(out)
    }
}
