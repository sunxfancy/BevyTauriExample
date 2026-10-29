//! Composite WebKit's transparent viewport into Bevy on Linux. GTK still owns
//! the real webview, so keyboard, pointer events, and Tauri IPC keep working.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::Manager;
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};

struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Resource)]
struct Overlay {
    latest: Arc<Mutex<Option<Frame>>>,
    pending: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    last_request: Instant,
    image: Handle<Image>,
}

pub fn install(app: &mut App) {
    app.insert_resource(Overlay {
        latest: Default::default(),
        pending: Default::default(),
        failed: Default::default(),
        last_request: Instant::now(),
        image: Handle::default(),
    })
    .add_systems(Startup, setup)
    .add_systems(Update, upload);
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut overlay: ResMut<Overlay>) {
    overlay.image = images.add(make_image(Frame {
        width: 1,
        height: 1,
        rgba: vec![0; 4],
    }));
    commands.spawn((
        ImageNode::new(overlay.image.clone()),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        GlobalZIndex(i32::MAX),
    ));
}

pub fn request_frame(app: &mut App) {
    let handle = app.world().non_send::<tauri::AppHandle>().clone();
    let Some(mut overlay) = app.world_mut().get_resource_mut::<Overlay>() else {
        return;
    };
    if overlay.last_request.elapsed() < Duration::from_millis(33)
        || overlay.pending.swap(true, Ordering::Relaxed)
    {
        return;
    }
    overlay.last_request = Instant::now();
    let latest = overlay.latest.clone();
    let pending = overlay.pending.clone();
    let failed = overlay.failed.clone();
    let Some(window) = handle.get_webview_window("main") else {
        overlay.pending.store(false, Ordering::Relaxed);
        return;
    };
    let result = window.with_webview(move |platform| {
        platform.inner().snapshot(
            SnapshotRegion::Visible,
            SnapshotOptions::TRANSPARENT_BACKGROUND,
            None::<&webkit2gtk::gio::Cancellable>,
            move |result| {
                let result = result
                    .map_err(|error| error.to_string())
                    .and_then(read_frame);
                match result {
                    Ok(frame) => {
                        *latest.lock().unwrap() = Some(frame);
                        failed.store(false, Ordering::Relaxed);
                    }
                    Err(error) => {
                        if !failed.swap(true, Ordering::Relaxed) {
                            eprintln!("WebKit overlay snapshot failed: {error}");
                        }
                    }
                }
                pending.store(false, Ordering::Relaxed);
            },
        );
    });
    if let Err(error) = result {
        overlay.pending.store(false, Ordering::Relaxed);
        if !overlay.failed.swap(true, Ordering::Relaxed) {
            eprintln!("WebKit overlay dispatch failed: {error}");
        }
    }
}

fn read_frame(surface: cairo::Surface) -> Result<Frame, String> {
    let surface = cairo::ImageSurface::try_from(surface).map_err(|_| "not an image surface")?;
    if surface.format() != cairo::Format::ARgb32 {
        return Err("expected ARGB32".into());
    }
    let (width, height, stride) = (
        surface.width() as usize,
        surface.height() as usize,
        surface.stride() as usize,
    );
    if width == 0 || height == 0 || stride < width * 4 {
        return Err("empty or invalid snapshot dimensions".into());
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    surface
        .with_data(|data| {
            for row in data.chunks(stride).take(height) {
                for bytes in row[..width * 4].chunks_exact(4) {
                    // Cairo is native-endian, premultiplied ARGB. Bevy UI expects
                    // straight-alpha RGBA, otherwise translucent edges darken.
                    let pixel = u32::from_ne_bytes(bytes.try_into().unwrap());
                    let alpha = (pixel >> 24) as u8;
                    let unpremultiply = |value: u32| {
                        if alpha == 0 {
                            0
                        } else {
                            ((value * 255 + alpha as u32 / 2) / alpha as u32).min(255) as u8
                        }
                    };
                    rgba.extend_from_slice(&[
                        unpremultiply((pixel >> 16) & 255),
                        unpremultiply((pixel >> 8) & 255),
                        unpremultiply(pixel & 255),
                        alpha,
                    ]);
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(Frame {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

fn make_image(frame: Frame) -> Image {
    Image::new(
        Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        frame.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

fn upload(overlay: Res<Overlay>, mut images: ResMut<Assets<Image>>) {
    let Some(frame) = overlay.latest.lock().unwrap().take() else {
        return;
    };
    // A mostly static page should not allocate/upload a GPU texture every poll.
    let Some(image) = images.get(&overlay.image) else {
        return;
    };
    if image.width() == frame.width
        && image.height() == frame.height
        && image.data.as_deref() == Some(frame.rgba.as_slice())
    {
        return;
    }
    if let Some(mut image) = images.get_mut(&overlay.image) {
        *image = make_image(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cairo_pixels_become_straight_alpha_rgba() {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 3, 1).unwrap();
        {
            let mut data = surface.data().unwrap();
            for (bytes, value) in
                data.chunks_exact_mut(4)
                    .zip([0xff123456u32, 0x80804020, 0x00000000])
            {
                bytes.copy_from_slice(&value.to_ne_bytes());
            }
        }
        surface.mark_dirty();
        let frame = read_frame(surface.as_ref().clone()).unwrap();
        assert_eq!((frame.width, frame.height), (3, 1));
        assert_eq!(
            frame.rgba,
            [0x12, 0x34, 0x56, 255, 255, 128, 64, 128, 0, 0, 0, 0]
        );
    }

    #[test]
    fn empty_snapshot_is_rejected_without_panicking() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 0, 0).unwrap();
        assert!(read_frame(surface.as_ref().clone()).is_err());
    }

    #[test]
    fn resizing_replaces_texture_dimensions_and_pixels() {
        let mut app = App::new();
        let mut images = Assets::<Image>::default();
        let image = images.add(make_image(Frame {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        }));
        let latest = Arc::new(Mutex::new(Some(Frame {
            width: 2,
            height: 3,
            rgba: vec![255; 24],
        })));
        app.insert_resource(images)
            .insert_resource(Overlay {
                latest: latest.clone(),
                pending: Default::default(),
                failed: Default::default(),
                last_request: Instant::now(),
                image: image.clone(),
            })
            .add_systems(Update, upload);
        app.update();
        let images = app.world().resource::<Assets<Image>>();
        let texture = images.get(&image).unwrap();
        assert_eq!((texture.width(), texture.height()), (2, 3));
        assert_eq!(texture.data.as_deref(), Some([255; 24].as_slice()));
        assert!(latest.lock().unwrap().is_none());
    }
}
