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

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust-Bevy App!", name)
}

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

        app.add_plugins((
            bevy::render::texture::ImagePlugin::default(),
            bevy::render::pipelined_rendering::PipelinedRenderingPlugin::default(),
            bevy::core_pipeline::CorePipelinePlugin::default(),
        ));
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

struct TauriPlugin {
    setup: Box<dyn Fn() -> tauri::App + Send + Sync>,
}

impl TauriPlugin {
    fn new<F>(setup: F) -> Self
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

static AVERAGE_FRAME_RATE: AtomicUsize = AtomicUsize::new(0);

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

#[tauri::command]
pub fn get_average_frame_rate() -> usize {
    AVERAGE_FRAME_RATE.load(Ordering::Relaxed)
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

// This function is called from the main thread to setup the Bevy app
pub fn setup_bevy() {
    // Configure Bevy to use the existing surface
    let mut app: App = App::new();
    app.insert_resource(ClearColor(Color::srgb_u8(0, 0, 0)));
    app.add_plugins((
        bevy::app::PanicHandlerPlugin::default(),
        bevy::log::LogPlugin::default(),
        bevy::core::TaskPoolPlugin::default(),
        bevy::core::TypeRegistrationPlugin::default(),
        bevy::core::FrameCountPlugin::default(),
        bevy::time::TimePlugin::default(),
        bevy::transform::TransformPlugin::default(),
        bevy::hierarchy::HierarchyPlugin::default(),
        bevy::diagnostic::DiagnosticsPlugin::default(),
        bevy::input::InputPlugin::default(),
        
    ));
    app.add_plugins(WindowPlugin {
        primary_window: Some(Window::default()),
        ..default()
    });
    app.add_plugins((
        bevy::a11y::AccessibilityPlugin::default(),
        bevy::asset::AssetPlugin::default(),
        bevy::scene::ScenePlugin::default(),
    ));

    // create tauri app
    app.add_plugins(TauriPlugin::new(|| {
        tauri::Builder::default()
            .invoke_handler(tauri::generate_handler![greet])
            .invoke_handler(tauri::generate_handler![get_average_frame_rate])
            .build(tauri::generate_context!())
            .expect("error while building tauri application")
    }));


    // App setup
    app.add_systems(Startup, setup)
        .insert_resource(AmbientLight {
            color: Color::WHITE,
            brightness: 150.0,
            ..default()
        });

    let _ = app.run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut animations: ResMut<Assets<AnimationClip>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // Camera
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-2.0, 2.5, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Light
    commands.spawn((
        PointLight {
            intensity: 500_000.0,
            ..default()
        },
        Transform::from_xyz(0.0, 2.5, 0.0),
    ));

    // Let's use the `Name` component to target entities. We can use anything we
    // like, but names are convenient.
    let planet = Name::new("planet");
    let orbit_controller = Name::new("orbit_controller");
    let satellite = Name::new("satellite");

    // Creating the animation
    let mut animation = AnimationClip::default();
    // A curve can modify a single part of a transform: here, the translation.
    let planet_animation_target_id = AnimationTargetId::from_name(&planet);
    animation.add_curve_to_target(
        planet_animation_target_id,
        AnimatableCurve::new(
            animated_field!(Transform::translation),
            UnevenSampleAutoCurve::new([0.0, 1.0, 2.0, 3.0, 4.0].into_iter().zip([
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(-1.0, 0.0, 1.0),
                Vec3::new(-1.0, 0.0, -1.0),
                Vec3::new(1.0, 0.0, -1.0),
                // in case seamless looping is wanted, the last keyframe should
                // be the same as the first one
                Vec3::new(1.0, 0.0, 1.0),
            ]))
            .expect("should be able to build translation curve because we pass in valid samples"),
        ),
    );
    // Or it can modify the rotation of the transform.
    // To find the entity to modify, the hierarchy will be traversed looking for
    // an entity with the right name at each level.
    let orbit_controller_animation_target_id =
        AnimationTargetId::from_names([planet.clone(), orbit_controller.clone()].iter());
    animation.add_curve_to_target(
        orbit_controller_animation_target_id,
        AnimatableCurve::new(
            animated_field!(Transform::rotation),
            UnevenSampleAutoCurve::new([0.0, 1.0, 2.0, 3.0, 4.0].into_iter().zip([
                Quat::IDENTITY,
                Quat::from_axis_angle(Vec3::Y, PI / 2.),
                Quat::from_axis_angle(Vec3::Y, PI / 2. * 2.),
                Quat::from_axis_angle(Vec3::Y, PI / 2. * 3.),
                Quat::IDENTITY,
            ]))
            .expect("Failed to build rotation curve"),
        ),
    );
    // If a curve in an animation is shorter than the other, it will not repeat
    // until all other curves are finished. In that case, another animation should
    // be created for each part that would have a different duration / period.
    let satellite_animation_target_id = AnimationTargetId::from_names(
        [planet.clone(), orbit_controller.clone(), satellite.clone()].iter(),
    );
    animation.add_curve_to_target(
        satellite_animation_target_id,
        AnimatableCurve::new(
            animated_field!(Transform::scale),
            UnevenSampleAutoCurve::new(
                [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0]
                    .into_iter()
                    .zip([
                        Vec3::splat(0.8),
                        Vec3::splat(1.2),
                        Vec3::splat(0.8),
                        Vec3::splat(1.2),
                        Vec3::splat(0.8),
                        Vec3::splat(1.2),
                        Vec3::splat(0.8),
                        Vec3::splat(1.2),
                        Vec3::splat(0.8),
                    ]),
            )
            .expect("Failed to build scale curve"),
        ),
    );
    // There can be more than one curve targeting the same entity path.
    animation.add_curve_to_target(
        AnimationTargetId::from_names(
            [planet.clone(), orbit_controller.clone(), satellite.clone()].iter(),
        ),
        AnimatableCurve::new(
            animated_field!(Transform::rotation),
            UnevenSampleAutoCurve::new([0.0, 1.0, 2.0, 3.0, 4.0].into_iter().zip([
                Quat::IDENTITY,
                Quat::from_axis_angle(Vec3::Y, PI / 2.),
                Quat::from_axis_angle(Vec3::Y, PI / 2. * 2.),
                Quat::from_axis_angle(Vec3::Y, PI / 2. * 3.),
                Quat::IDENTITY,
            ]))
            .expect("should be able to build translation curve because we pass in valid samples"),
        ),
    );

    // Create the animation graph
    let (graph, animation_index) = AnimationGraph::from_clip(animations.add(animation));

    // Create the animation player, and set it to repeat
    let mut player = AnimationPlayer::default();
    player.play(animation_index).repeat();

    // Create the scene that will be animated
    // First entity is the planet
    let planet_entity = commands
        .spawn((
            Mesh3d(meshes.add(Sphere::default())),
            MeshMaterial3d(materials.add(Color::srgb(0.8, 0.7, 0.6))),
            // Add the animation graph and player
            planet,
            AnimationGraphHandle(graphs.add(graph)),
            player,
        ))
        .id();
    commands
        .entity(planet_entity)
        .insert(AnimationTarget {
            id: planet_animation_target_id,
            player: planet_entity,
        })
        .with_children(|p| {
            // This entity is just used for animation, but doesn't display anything
            p.spawn((
                Transform::default(),
                Visibility::default(),
                orbit_controller,
                AnimationTarget {
                    id: orbit_controller_animation_target_id,
                    player: planet_entity,
                },
            ))
            .with_children(|p| {
                // The satellite, placed at a distance of the planet
                p.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.5, 0.5, 0.5))),
                    MeshMaterial3d(materials.add(Color::srgb(0.3, 0.9, 0.3))),
                    Transform::from_xyz(1.5, 0.0, 0.0),
                    AnimationTarget {
                        id: satellite_animation_target_id,
                        player: planet_entity,
                    },
                    satellite,
                ));
            });
        });
}
