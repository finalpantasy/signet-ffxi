// Generated COM signatures only; source ABI: frozen d3d8 reference header.
forward!(root_register_software_device, 3, i32, (argument_0: Object));
forward!(root_get_adapter_count, 4, u32, ());
forward!(root_get_adapter_identifier, 5, i32, (argument_0: u32, argument_1: u32, argument_2: Object));
forward!(root_get_adapter_mode_count, 6, u32, (argument_0: u32));
forward!(root_enum_adapter_modes, 7, i32, (argument_0: u32, argument_1: u32, argument_2: Object));
forward!(root_get_adapter_display_mode, 8, i32, (argument_0: u32, argument_1: Object));
forward!(root_check_device_type, 9, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: i32));
forward!(root_check_device_format, 10, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: u32, argument_5: u32));
forward!(root_check_device_multi_sample_type, 11, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: i32, argument_4: u32));
forward!(root_check_depth_stencil_match, 12, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: u32));
forward!(root_get_device_caps, 13, i32, (argument_0: u32, argument_1: u32, argument_2: Object));
forward!(root_get_adapter_monitor, 14, Object, (argument_0: u32));
fn root_table() -> &'static [usize; 16] {
    static TABLE: OnceLock<[usize; 16]> = OnceLock::new();
    TABLE.get_or_init(|| [
        query_interface as *const () as usize,
        add_ref as *const () as usize,
        release as *const () as usize,
        root_register_software_device as *const () as usize,
        root_get_adapter_count as *const () as usize,
        root_get_adapter_identifier as *const () as usize,
        root_get_adapter_mode_count as *const () as usize,
        root_enum_adapter_modes as *const () as usize,
        root_get_adapter_display_mode as *const () as usize,
        root_check_device_type as *const () as usize,
        root_check_device_format as *const () as usize,
        root_check_device_multi_sample_type as *const () as usize,
        root_check_depth_stencil_match as *const () as usize,
        root_get_device_caps as *const () as usize,
        root_get_adapter_monitor as *const () as usize,
        create_device as *const () as usize,
    ])
}
forward!(device_test_cooperative_level, 3, i32, ());
forward!(device_get_available_texture_mem, 4, u32, ());
forward!(device_resource_manager_discard_bytes, 5, i32, (argument_0: u32));
forward!(device_get_device_caps, 7, i32, (argument_0: Object));
forward!(device_get_display_mode, 8, i32, (argument_0: Object));
forward!(device_get_creation_parameters, 9, i32, (argument_0: Object));
forward!(device_set_cursor_properties, 10, i32, (argument_0: u32, argument_1: u32, argument_2: Object));
forward!(device_set_cursor_position, 11, (), (argument_0: u32, argument_1: u32, argument_2: u32));
forward!(device_show_cursor, 12, i32, (argument_0: i32));
forward!(device_create_additional_swap_chain, 13, i32, (argument_0: Object, argument_1: Object));
forward!(device_reset, 14, i32, (argument_0: Object));
forward!(device_get_back_buffer, 16, i32, (argument_0: u32, argument_1: u32, argument_2: Object));
forward!(device_get_raster_status, 17, i32, (argument_0: Object));
forward!(device_set_gamma_ramp, 18, (), (argument_0: u32, argument_1: Object));
forward!(device_get_gamma_ramp, 19, (), (argument_0: Object));
forward!(device_create_texture, 20, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: u32, argument_5: u32, argument_6: Object));
forward!(device_create_volume_texture, 21, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: u32, argument_5: u32, argument_6: u32, argument_7: Object));
forward!(device_create_cube_texture, 22, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: u32, argument_5: Object));
forward!(device_create_vertex_buffer, 23, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: Object));
forward!(device_create_index_buffer, 24, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: Object));
forward!(device_create_render_target, 25, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: i32, argument_5: Object));
forward!(device_create_depth_stencil_surface, 26, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: Object));
forward!(device_create_image_surface, 27, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: Object));
forward!(device_copy_rects, 28, i32, (argument_0: Object, argument_1: Object, argument_2: u32, argument_3: Object, argument_4: Object));
forward!(device_update_texture, 29, i32, (argument_0: Object, argument_1: Object));
forward!(device_get_front_buffer, 30, i32, (argument_0: Object));
forward!(device_set_render_target, 31, i32, (argument_0: Object, argument_1: Object));
forward!(device_get_render_target, 32, i32, (argument_0: Object));
forward!(device_get_depth_stencil_surface, 33, i32, (argument_0: Object));
forward!(device_begin_scene, 34, i32, ());
forward!(device_end_scene, 35, i32, ());
forward!(device_clear, 36, i32, (argument_0: u32, argument_1: Object, argument_2: u32, argument_3: u32, argument_4: f32, argument_5: u32));
forward!(device_set_transform, 37, i32, (argument_0: u32, argument_1: Object));
forward!(device_get_transform, 38, i32, (argument_0: u32, argument_1: Object));
forward!(device_multiply_transform, 39, i32, (argument_0: u32, argument_1: Object));
forward!(device_set_viewport, 40, i32, (argument_0: Object));
forward!(device_get_viewport, 41, i32, (argument_0: Object));
forward!(device_set_material, 42, i32, (argument_0: Object));
forward!(device_get_material, 43, i32, (argument_0: Object));
forward!(device_set_light, 44, i32, (argument_0: u32, argument_1: Object));
forward!(device_get_light, 45, i32, (argument_0: u32, argument_1: Object));
forward!(device_light_enable, 46, i32, (argument_0: u32, argument_1: i32));
forward!(device_get_light_enable, 47, i32, (argument_0: u32, argument_1: Object));
forward!(device_set_clip_plane, 48, i32, (argument_0: u32, argument_1: Object));
forward!(device_get_clip_plane, 49, i32, (argument_0: u32, argument_1: Object));
forward!(device_set_render_state, 50, i32, (argument_0: u32, argument_1: u32));
forward!(device_get_render_state, 51, i32, (argument_0: u32, argument_1: Object));
forward!(device_begin_state_block, 52, i32, ());
forward!(device_end_state_block, 53, i32, (argument_0: Object));
forward!(device_apply_state_block, 54, i32, (argument_0: u32));
forward!(device_capture_state_block, 55, i32, (argument_0: u32));
forward!(device_delete_state_block, 56, i32, (argument_0: u32));
forward!(device_create_state_block, 57, i32, (argument_0: u32, argument_1: Object));
forward!(device_set_clip_status, 58, i32, (argument_0: Object));
forward!(device_get_clip_status, 59, i32, (argument_0: Object));
forward!(device_get_texture, 60, i32, (argument_0: u32, argument_1: Object));
forward!(device_set_texture, 61, i32, (argument_0: u32, argument_1: Object));
forward!(device_get_texture_stage_state, 62, i32, (argument_0: u32, argument_1: u32, argument_2: Object));
forward!(device_set_texture_stage_state, 63, i32, (argument_0: u32, argument_1: u32, argument_2: u32));
forward!(device_validate_device, 64, i32, (argument_0: Object));
forward!(device_get_info, 65, i32, (argument_0: u32, argument_1: Object, argument_2: u32));
forward!(device_set_palette_entries, 66, i32, (argument_0: u32, argument_1: Object));
forward!(device_get_palette_entries, 67, i32, (argument_0: u32, argument_1: Object));
forward!(device_set_current_texture_palette, 68, i32, (argument_0: u32));
forward!(device_get_current_texture_palette, 69, i32, (argument_0: Object));
forward!(device_draw_primitive, 70, i32, (argument_0: u32, argument_1: u32, argument_2: u32));
forward!(device_draw_indexed_primitive, 71, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: u32));
forward!(device_draw_primitive_u_p, 72, i32, (argument_0: u32, argument_1: u32, argument_2: Object, argument_3: u32));
forward!(device_draw_indexed_primitive_u_p, 73, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: u32, argument_4: Object, argument_5: u32, argument_6: Object, argument_7: u32));
forward!(device_process_vertices, 74, i32, (argument_0: u32, argument_1: u32, argument_2: u32, argument_3: Object, argument_4: u32));
forward!(device_create_vertex_shader, 75, i32, (argument_0: Object, argument_1: Object, argument_2: Object, argument_3: u32));
forward!(device_set_vertex_shader, 76, i32, (argument_0: u32));
forward!(device_get_vertex_shader, 77, i32, (argument_0: Object));
forward!(device_delete_vertex_shader, 78, i32, (argument_0: u32));
forward!(device_set_vertex_shader_constant, 79, i32, (argument_0: u32, argument_1: Object, argument_2: u32));
forward!(device_get_vertex_shader_constant, 80, i32, (argument_0: u32, argument_1: Object, argument_2: u32));
forward!(device_get_vertex_shader_declaration, 81, i32, (argument_0: u32, argument_1: Object, argument_2: Object));
forward!(device_get_vertex_shader_function, 82, i32, (argument_0: u32, argument_1: Object, argument_2: Object));
forward!(device_set_stream_source, 83, i32, (argument_0: u32, argument_1: Object, argument_2: u32));
forward!(device_get_stream_source, 84, i32, (argument_0: u32, argument_1: Object, argument_2: Object));
forward!(device_set_indices, 85, i32, (argument_0: Object, argument_1: u32));
forward!(device_get_indices, 86, i32, (argument_0: Object, argument_1: Object));
forward!(device_create_pixel_shader, 87, i32, (argument_0: Object, argument_1: Object));
forward!(device_set_pixel_shader, 88, i32, (argument_0: u32));
forward!(device_get_pixel_shader, 89, i32, (argument_0: Object));
forward!(device_delete_pixel_shader, 90, i32, (argument_0: u32));
forward!(device_set_pixel_shader_constant, 91, i32, (argument_0: u32, argument_1: Object, argument_2: u32));
forward!(device_get_pixel_shader_constant, 92, i32, (argument_0: u32, argument_1: Object, argument_2: u32));
forward!(device_get_pixel_shader_function, 93, i32, (argument_0: u32, argument_1: Object, argument_2: Object));
forward!(device_draw_rect_patch, 94, i32, (argument_0: u32, argument_1: Object, argument_2: Object));
forward!(device_draw_tri_patch, 95, i32, (argument_0: u32, argument_1: Object, argument_2: Object));
forward!(device_delete_patch, 96, i32, (argument_0: u32));
fn device_table() -> &'static [usize; 97] {
    static TABLE: OnceLock<[usize; 97]> = OnceLock::new();
    TABLE.get_or_init(|| [
        query_interface as *const () as usize,
        add_ref as *const () as usize,
        release as *const () as usize,
        device_test_cooperative_level as *const () as usize,
        device_get_available_texture_mem as *const () as usize,
        device_resource_manager_discard_bytes as *const () as usize,
        get_parent as *const () as usize,
        device_get_device_caps as *const () as usize,
        device_get_display_mode as *const () as usize,
        device_get_creation_parameters as *const () as usize,
        device_set_cursor_properties as *const () as usize,
        device_set_cursor_position as *const () as usize,
        device_show_cursor as *const () as usize,
        device_create_additional_swap_chain as *const () as usize,
        device_reset as *const () as usize,
        present as *const () as usize,
        device_get_back_buffer as *const () as usize,
        device_get_raster_status as *const () as usize,
        device_set_gamma_ramp as *const () as usize,
        device_get_gamma_ramp as *const () as usize,
        device_create_texture as *const () as usize,
        device_create_volume_texture as *const () as usize,
        device_create_cube_texture as *const () as usize,
        device_create_vertex_buffer as *const () as usize,
        device_create_index_buffer as *const () as usize,
        device_create_render_target as *const () as usize,
        device_create_depth_stencil_surface as *const () as usize,
        device_create_image_surface as *const () as usize,
        device_copy_rects as *const () as usize,
        device_update_texture as *const () as usize,
        device_get_front_buffer as *const () as usize,
        device_set_render_target as *const () as usize,
        device_get_render_target as *const () as usize,
        device_get_depth_stencil_surface as *const () as usize,
        device_begin_scene as *const () as usize,
        device_end_scene as *const () as usize,
        device_clear as *const () as usize,
        device_set_transform as *const () as usize,
        device_get_transform as *const () as usize,
        device_multiply_transform as *const () as usize,
        device_set_viewport as *const () as usize,
        device_get_viewport as *const () as usize,
        device_set_material as *const () as usize,
        device_get_material as *const () as usize,
        device_set_light as *const () as usize,
        device_get_light as *const () as usize,
        device_light_enable as *const () as usize,
        device_get_light_enable as *const () as usize,
        device_set_clip_plane as *const () as usize,
        device_get_clip_plane as *const () as usize,
        device_set_render_state as *const () as usize,
        device_get_render_state as *const () as usize,
        device_begin_state_block as *const () as usize,
        device_end_state_block as *const () as usize,
        device_apply_state_block as *const () as usize,
        device_capture_state_block as *const () as usize,
        device_delete_state_block as *const () as usize,
        device_create_state_block as *const () as usize,
        device_set_clip_status as *const () as usize,
        device_get_clip_status as *const () as usize,
        device_get_texture as *const () as usize,
        device_set_texture as *const () as usize,
        device_get_texture_stage_state as *const () as usize,
        device_set_texture_stage_state as *const () as usize,
        device_validate_device as *const () as usize,
        device_get_info as *const () as usize,
        device_set_palette_entries as *const () as usize,
        device_get_palette_entries as *const () as usize,
        device_set_current_texture_palette as *const () as usize,
        device_get_current_texture_palette as *const () as usize,
        device_draw_primitive as *const () as usize,
        device_draw_indexed_primitive as *const () as usize,
        device_draw_primitive_u_p as *const () as usize,
        device_draw_indexed_primitive_u_p as *const () as usize,
        device_process_vertices as *const () as usize,
        device_create_vertex_shader as *const () as usize,
        device_set_vertex_shader as *const () as usize,
        device_get_vertex_shader as *const () as usize,
        device_delete_vertex_shader as *const () as usize,
        device_set_vertex_shader_constant as *const () as usize,
        device_get_vertex_shader_constant as *const () as usize,
        device_get_vertex_shader_declaration as *const () as usize,
        device_get_vertex_shader_function as *const () as usize,
        device_set_stream_source as *const () as usize,
        device_get_stream_source as *const () as usize,
        device_set_indices as *const () as usize,
        device_get_indices as *const () as usize,
        device_create_pixel_shader as *const () as usize,
        device_set_pixel_shader as *const () as usize,
        device_get_pixel_shader as *const () as usize,
        device_delete_pixel_shader as *const () as usize,
        device_set_pixel_shader_constant as *const () as usize,
        device_get_pixel_shader_constant as *const () as usize,
        device_get_pixel_shader_function as *const () as usize,
        device_draw_rect_patch as *const () as usize,
        device_draw_tri_patch as *const () as usize,
        device_delete_patch as *const () as usize,
    ])
}
