use bevy::app::Plugin;
use bevy::app::PluginsState;

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
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use std::time::{Duration, Instant};
use tauri::RunEvent;
use tauri::{async_runtime::block_on, Manager};

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
        trace_startup("building Tauri application");
        let tauri_app = (self.setup)();
        trace_startup("Tauri application built");

        app.insert_non_send(tauri_app.handle().clone());
        app.insert_non_send(tauri_app);
        app.set_runner(run_tauri_app);
    }
}

fn trace_startup(stage: &str) {
    if cfg!(feature = "ci-smoke") {
        eprintln!("Desktop startup: {stage}");
    }
}

pub static AVERAGE_FRAME_RATE: AtomicUsize = AtomicUsize::new(0);

#[cfg(not(target_os = "macos"))]
fn run_tauri_app(app: App) -> AppExit {
    let app = Rc::new(RefCell::new(app));
    let mut tauri_app = app
        .borrow_mut()
        .world_mut()
        .remove_non_send::<tauri::App>()
        .unwrap();

    let target_frame_duration = Duration::from_secs_f64(1.0 / 60.0); // 60Hz
    let mut frame_count = 0;
    let mut last_second = Instant::now();

    trace_startup("entering Tauri event loop");
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

        #[cfg(target_os = "linux")]
        crate::linux_overlay::request_frame(&mut app.borrow_mut());
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

// AppKit must keep running between frames so WKWebView can service its main
// dispatch queue. Repeated run_iteration()/NSApplication.stop calls can starve
// page loading on macOS even while Bevy continues rendering.
#[cfg(target_os = "macos")]
fn run_tauri_app(mut app: App) -> AppExit {
    let tauri_app = app.world_mut().remove_non_send::<tauri::App>().unwrap();
    let app = Rc::new(RefCell::new(app));
    let frame_interval = Duration::from_secs_f64(1.0 / 60.0);
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let wake_running = running.clone();
    let handle = tauri_app.handle().clone();
    // Wake the otherwise waiting native loop without sleeping on its thread.
    std::thread::spawn(move || {
        while wake_running.load(Ordering::Relaxed) {
            std::thread::sleep(frame_interval);
            if handle.run_on_main_thread(|| {}).is_err() {
                break;
            }
        }
    });
    let mut last_frame = Instant::now();
    let mut last_second = Instant::now();
    let mut frame_count = 0;
    trace_startup("entering continuous Tauri event loop");
    tauri_app.run(move |handle, event| {
        if matches!(event, RunEvent::Exit) {
            running.store(false, Ordering::Relaxed);
        }
        if matches!(event, RunEvent::MainEventsCleared)
            && app.borrow().plugins_state() == PluginsState::Cleaned
            && last_frame.elapsed() >= frame_interval
        {
            last_frame = Instant::now();
            app.borrow_mut().update();
            frame_count += 1;
            if last_second.elapsed() >= Duration::from_secs(1) {
                AVERAGE_FRAME_RATE.store(frame_count, Ordering::Relaxed);
                frame_count = 0;
                last_second = Instant::now();
            }
        }
        handle_tauri_events(handle, event, app.borrow_mut());
    });
    AppExit::Success
}

fn handle_tauri_events(app_handle: &tauri::AppHandle, event: RunEvent, mut app: RefMut<'_, App>) {
    match event {
        tauri::RunEvent::Ready => handle_ready_event(app_handle, app),
        tauri::RunEvent::ExitRequested { .. } => {}
        tauri::RunEvent::WindowEvent { event, .. }
            if app.plugins_state() == PluginsState::Cleaned =>
        {
            handle_window_event(event, app)
        }
        tauri::RunEvent::MainEventsCleared => {}
        _ => (),
    }
}

fn handle_ready_event(app_handle: &tauri::AppHandle, mut app: RefMut<'_, App>) {
    if app.plugins_state() != PluginsState::Cleaned {
        trace_startup("Tauri Ready received");
        let window = app_handle.get_webview_window("main").unwrap();
        // Tauri owns the native window and event loop. Initialize Bevy's renderer
        // against that window before adding the standard rendering plugins.
        let wrapper = WindowWrapper::new(window.clone());
        let raw_handle = RawHandleWrapper::new(&wrapper).expect("invalid Tauri window handle");
        let holder = RawHandleWrapperHolder(Arc::new(Mutex::new(Some(raw_handle.clone()))));
        let settings = WgpuSettings::default();
        trace_startup("initializing GPU renderer");
        let resources = block_on(initialize_renderer(
            settings.backends.expect("no rendering backend enabled"),
            Some(holder.clone()),
            &settings,
        ));
        trace_startup("GPU renderer initialized");
        let size = window.inner_size().expect("cannot read window size");
        let scale_factor = window
            .scale_factor()
            .expect("cannot read window scale factor");
        let mut resolution = WindowResolution::new(size.width, size.height);
        resolution.set_scale_factor(scale_factor as f32);

        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        resolution,
                        ..default()
                    }),
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Manual(Box::new(resources)),
                    ..default()
                }),
        );
        trace_startup("Bevy plugins built");
        let world = app.world_mut();
        let mut windows = world.query_filtered::<Entity, With<bevy::window::PrimaryWindow>>();
        let entity = windows.single(world).expect("missing primary window");
        world.entity_mut(entity).insert((raw_handle, holder));
        #[cfg(target_os = "linux")]
        crate::linux_overlay::install(&mut app);
        // wait for bevy to be ready

        while app.plugins_state() != PluginsState::Ready {
            tick_global_task_pools_on_main_thread();
        }

        trace_startup("Bevy plugins ready");
        app.finish();
        app.cleanup();
        trace_startup("Bevy initialization complete");
    }
}

fn handle_window_event(event: tauri::WindowEvent, app: RefMut<'_, App>) {
    match event {
        tauri::WindowEvent::Resized(size) => handle_window_resize(size, app),
        tauri::WindowEvent::ScaleFactorChanged {
            scale_factor,
            new_inner_size,
            ..
        } => handle_window_factor_change(scale_factor, new_inner_size, app),
        _ => (),
    }
}

fn handle_window_resize(size: tauri::PhysicalSize<u32>, mut app: RefMut<'_, App>) {
    let mut event_writer_system_state: SystemState<(
        MessageWriter<WindowResized>,
        Query<(Entity, &mut Window)>,
    )> = SystemState::new(app.world_mut());

    let (mut window_resized, mut window_query) = event_writer_system_state
        .get_mut(app.world_mut())
        .expect("window messages must be initialized");

    for (entity, mut window) in window_query.iter_mut() {
        window
            .resolution
            .set_physical_resolution(size.width, size.height);
        window_resized.write(WindowResized {
            window: entity,
            width: window.width(),
            height: window.height(),
        });
    }
}

fn handle_window_factor_change(
    scale_factor: f64,
    new_inner_size: tauri::PhysicalSize<u32>,
    mut app: RefMut<'_, App>,
) {
    let mut event_writer_system_state: SystemState<(
        MessageWriter<WindowResized>,
        MessageWriter<WindowScaleFactorChanged>,
        Query<(Entity, &mut Window)>,
    )> = SystemState::new(app.world_mut());

    let (mut window_resized, mut window_scale_factor_changed, mut window_query) =
        event_writer_system_state
            .get_mut(app.world_mut())
            .expect("window messages must be initialized");

    for (entity, mut window) in window_query.iter_mut() {
        window.resolution.set_scale_factor(scale_factor as f32);
        window
            .resolution
            .set_physical_resolution(new_inner_size.width, new_inner_size.height);
        window_scale_factor_changed.write(WindowScaleFactorChanged {
            window: entity,
            scale_factor,
        });
        window_resized.write(WindowResized {
            window: entity,
            width: window.width(),
            height: window.height(),
        });
    }
}
