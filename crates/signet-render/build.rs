//! Compiles shaders/zone.wgsl to one SPIR-V module per entry point for the Vulkan backend.
use std::{env, fs, path::PathBuf};

use naga::{
    ShaderStage,
    back::spv::{Options, PipelineOptions, WriterFlags},
    valid::{Capabilities, ValidationFlags, Validator},
};

fn main() {
    println!("cargo:rerun-if-changed=shaders/zone.wgsl");
    println!("cargo:rerun-if-changed=build.rs");
    let src = fs::read_to_string("shaders/zone.wgsl").expect("read zone.wgsl");
    let module = match naga::front::wgsl::parse_str(&src) {
        Ok(m) => m,
        Err(e) => panic!("zone.wgsl: {}", e.emit_to_string(&src)),
    };
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty()).validate(&module).expect("zone.wgsl validation");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    // No coordinate-space adjustment: the backend flips Y with a negative-height viewport.
    let options = Options { lang_version: (1, 3), flags: WriterFlags::empty(), ..Default::default() };
    let entries = [
        ("VSMain", ShaderStage::Vertex),
        ("VSSkin", ShaderStage::Vertex),
        ("PSOpaque", ShaderStage::Fragment),
        ("PSCutout", ShaderStage::Fragment),
        ("PSBlend", ShaderStage::Fragment),
    ];
    for (name, stage) in entries {
        let words = naga::back::spv::write_vec(&module, &info, &options, Some(&PipelineOptions { shader_stage: stage, entry_point: name.into() }))
            .unwrap_or_else(|e| panic!("SPIR-V {name}: {e}"));
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        fs::write(out.join(format!("{name}.spv")), bytes).unwrap();
    }
}
