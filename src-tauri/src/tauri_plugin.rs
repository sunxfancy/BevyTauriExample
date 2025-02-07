use bevy::animation::{animated_field, AnimationTarget, AnimationTargetId};
use bevy::app::{plugin_group, Plugin};
use bevy::app::{PluginsState, ScheduleRunnerPlugin};
use bevy::ecs::entity::EntityHashMap;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use bevy::render::renderer::*;
use bevy::render::settings::{RenderCreation, WgpuSettings};
use bevy::render::RenderPlugin;
use bevy::tasks::tick_global_task_pools_on_main_thread;
use bevy::window::{
    RawHandleWrapper, RawHandleWrapperHolder, WindowResized, WindowResolution,
    WindowScaleFactorChanged, WindowWrapper,
};
use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{async_runtime::block_on, Manager};
use tauri::{EventLoopMessage, RunEvent, WebviewWindow, Wry};
use wgpu::RequestAdapterOptions;
use std::sync::atomic::{AtomicUsize, Ordering};



struct CustomRendererPlugin {
    webview_window: WebviewWindow,
}

impl Plugin for CustomRendererPlugin {
    fn build(&self, app: &mut App) {
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(&self.webview_window).unwrap();

        let (device, queue, adapter_info, adapter) = block_on(initialize_renderer(
            &instance,
            &WgpuSettings::default(),
            &RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            },
        ));

        app.add_plugins(RenderPlugin {
            render_creation: RenderCreation::Manual(
                device,
                queue,
                adapter_info,
                adapter,
                RenderInstance(Arc::new(WgpuWrapper::new(instance))),
            ),
            ..default()
        });
    }
}

fn create_window_handle(
    mut commands: Commands,
    query: Query<(Entity, Option<&'static RawHandleWrapperHolder>)>,
    tauri_app: NonSend<tauri::AppHandle>,
) {
    let tauri_window = tauri_app.get_webview_window("main").unwrap();
    let window_wrapper = WindowWrapper::new(tauri_window);

    for (entity, handle_holder) in query.iter() {
        if let Ok(handle_wrapper) = RawHandleWrapper::new(&window_wrapper) {
            commands.entity(entity).insert(handle_wrapper.clone());

            if let Some(handle_holder) = handle_holder {
                *handle_holder.0.lock().unwrap() = Some(handle_wrapper);
            }
        }
    }
}

pub struct TauriPlugin {
    setup: Box<dyn Fn() -> tauri::App + Send + Sync>,
}

impl TauriPlugin {
    pub fn new<F>(setup: F) -> Self
    where
        F: Fn() -> tauri::App + Send + Sync + 'static,
    {

        Self {
            setup: Box::new(setup),
        }
    }
}

impl Plugin for TauriPlugin {
    fn build(&self, app: &mut App) {
        let tauri_app = (self.setup)();

        app.add_systems(Startup, create_window_handle);
        app.insert_non_send_resource(tauri_app.handle().clone());
        app.insert_non_send_resource(tauri_app);
        app.set_runner(run_tauri_app);
    }
}

pub static AVERAGE_FRAME_RATE: AtomicUsize = AtomicUsize::new(0);

fn run_tauri_app(app: App) -> AppExit {
    let app = Rc::new(RefCell::new(app));
    let mut tauri_app = app
        .borrow_mut()
        .world_mut()
        .remove_non_send_resource::<tauri::App>()
        .unwrap();

    let target_frame_duration = Duration::from_secs_f64(1.0 / 60.0); // 60Hz
    let mut frame_count = 0;
    let mut last_second = Instant::now();

    loop {
        let frame_start = Instant::now(); 

        let app_clone = app.clone();
        tauri_app.run_iteration(move |app_handle, event: RunEvent| {
            handle_tauri_events(app_handle, event, app_clone.borrow_mut());
        });

        if tauri_app.webview_windows().is_empty() {
            bevy::log::info!("cleanup_before_exit");
            tauri_app.cleanup_before_exit();
            break;
        }

        app.borrow_mut().update();
        let frame_duration = frame_start.elapsed();
        if frame_duration < target_frame_duration {
            std::thread::sleep(target_frame_duration - frame_duration);
        }

        frame_count += 1;

        if last_second.elapsed() >= Duration::from_secs(1) {
            AVERAGE_FRAME_RATE.store(frame_count, Ordering::Relaxed);
            frame_count = 0;
            last_second = Instant::now();
        }
    }

    AppExit::Success
}


fn handle_tauri_events(app_handle: &tauri::AppHandle, event: RunEvent, mut app: RefMut<'_, App>) {
    if app.plugins_state() != PluginsState::Cleaned {
        if app.plugins_state() != PluginsState::Ready {
            tick_global_task_pools_on_main_thread();
        }
    }

    match event {
        tauri::RunEvent::Ready => handle_ready_event(app_handle, app),
        tauri::RunEvent::ExitRequested { api, .. } => {}
        tauri::RunEvent::WindowEvent { label, event, .. } => handle_window_event(event, app),
        tauri::RunEvent::MainEventsCleared => {}
        _ => (),
    }
}

fn handle_ready_event(app_handle: &tauri::AppHandle, mut app: RefMut<'_, App>) {
    if app.plugins_state() != PluginsState::Cleaned {
        let window = app_handle.get_webview_window("main").unwrap();
        app.add_plugins(CustomRendererPlugin {
            webview_window: window,
        });

        app.add_plugins((
            bevy::render::texture::ImagePlugin::default(),
            bevy::render::pipelined_rendering::PipelinedRenderingPlugin::default(),
            bevy::core_pipeline::CorePipelinePlugin::default(),
            bevy::sprite::SpritePlugin::default(),
            bevy::text::TextPlugin::default(),
            bevy::ui::UiPlugin::default(),
            bevy::pbr::PbrPlugin::default(),
            bevy::gltf::GltfPlugin::default(),
            bevy::audio::AudioPlugin::default(),
            bevy::gilrs::GilrsPlugin::default(),
            bevy::animation::AnimationPlugin::default(),
            bevy::gizmos::GizmoPlugin::default(),
            bevy::state::app::StatesPlugin::default(),
            bevy::picking::DefaultPickingPlugins::default(),
        ));
        // wait for bevy to be ready

        while app.plugins_state() != PluginsState::Ready {
            tick_global_task_pools_on_main_thread();
        }

        app.finish();
        app.cleanup();
    }
}

fn handle_window_event(event: tauri::WindowEvent, app: RefMut<'_, App>) {
    match event {
        tauri::WindowEvent::Resized(size) => handle_window_resize(size, app),
        tauri::WindowEvent::ScaleFactorChanged {
            scale_factor,
            new_inner_size,
            ..
        } => {}
        _ => (),
    }
}

fn handle_window_resize(size: tauri::PhysicalSize<u32>, mut app: RefMut<'_, App>) {
    let mut event_writer_system_state: SystemState<(
        EventWriter<WindowResized>,
        Query<(Entity, &mut Window)>,
    )> = SystemState::new(app.world_mut());

    let (mut window_resized, mut window_query) = event_writer_system_state.get_mut(app.world_mut());

    for (entity, mut window) in window_query.iter_mut() {
        window.resolution = WindowResolution::new(size.width as f32, size.height as f32);
        window_resized.send(WindowResized {
            window: entity,
            width: size.width as f32,
            height: size.height as f32,
        });
    }
}

