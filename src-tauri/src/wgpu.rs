use std::{borrow::Cow, sync::Mutex};
use tauri::{async_runtime::block_on, Manager, RunEvent, WindowEvent};


// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_average_frame_rate() -> usize {
    0
}


pub fn setup_wgpu() {
    tauri::Builder::default()
            .setup(move |app| {
                return setup_wgpu_handler(app);
            })
            .invoke_handler(tauri::generate_handler![greet])
            .invoke_handler(tauri::generate_handler![get_average_frame_rate])
            .build(crate::generate_tauri_context())
            .expect("error while building tauri application")
            .run(move |app_handle, event: RunEvent| {
                wgpu_callback(app_handle, event);
            });
}

pub fn setup_wgpu_handler(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let window = app.get_webview_window("main").unwrap();
            let size = window.inner_size()?;

            let instance = wgpu::Instance::default();

            let surface = instance.create_surface(window).unwrap();
            let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                // Request an adapter which can render to our surface
                compatible_surface: Some(&surface),
            }))
            .expect("Failed to find an appropriate adapter");

            // Create the logical device and command queue
            let (device, queue) = block_on(
                adapter.request_device(
                    &wgpu::DeviceDescriptor {
                        label: None,
                        memory_hints: wgpu::MemoryHints::default(),
                        required_features: wgpu::Features::empty(),
                        // Make sure we use the texture resolution limits from the adapter, so we can support images the size of the swapchain.
                        required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                            .using_resolution(adapter.limits()),
                    },
                    None,
                ),
            )
            .expect("Failed to create device");

            // Load the shaders from disk
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(
                    r#"
@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(in_vertex_index) - 1);
    let y = f32(i32(in_vertex_index & 1u) * 2 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
"#,
                )),
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });

            let swapchain_capabilities = surface.get_capabilities(&adapter);
            let swapchain_format = swapchain_capabilities.formats[0];

            let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                cache: None,
                label: None,
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(swapchain_format.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            });

            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: swapchain_format,
                width: size.width,
                height: size.height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: swapchain_capabilities.alpha_modes[0],
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };

            surface.configure(&device, &config);

            app.manage(surface);
            app.manage(render_pipeline);
            app.manage(device);
            app.manage(queue);
            app.manage(Mutex::new(config));

    Ok(())
} 


pub fn wgpu_callback(app_handle: &tauri::AppHandle, event: RunEvent) {
    match event {
        RunEvent::WindowEvent {
            label: _,
            event: WindowEvent::Resized(size),
            ..
        } => {
            let config = app_handle.state::<Mutex<wgpu::SurfaceConfiguration>>();
            let surface = app_handle.state::<wgpu::Surface>();
            let device = app_handle.state::<wgpu::Device>();

            let mut config = config.lock().unwrap();
            config.width = if size.width > 0 { size.width } else { 1 };
            config.height = if size.height > 0 { size.height } else { 1 };
            surface.configure(&device, &config)

            // TODO: Request redraw on macos (not exposed in tauri yet).
        }
        RunEvent::MainEventsCleared => {
            println!("MainEventsCleared");

            let surface = app_handle.state::<wgpu::Surface>();
            let render_pipeline = app_handle.state::<wgpu::RenderPipeline>();
            let device = app_handle.state::<wgpu::Device>();
            let queue = app_handle.state::<wgpu::Queue>();

            let frame = surface
                .get_current_texture()
                .expect("Failed to acquire next swap chain texture");
            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                rpass.set_pipeline(&render_pipeline);
                rpass.draw(0..3, 0..1);
            }

            queue.submit(Some(encoder.finish()));
            frame.present();
        }
        _ => (),
    }
}