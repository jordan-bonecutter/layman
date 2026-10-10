//! Direct Wayland screencopy, using shared-memory buffers. No screenshot subprocess.
use anyhow::{Context, Result, bail, ensure};
use eframe::egui::{Color32, ColorImage};
use std::{
    fs::File,
    os::{
        fd::AsFd,
        unix::{fs::FileExt, net::UnixStream},
    },
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop,
    protocol::{wl_buffer, wl_callback, wl_output, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1 as frame, zwlr_screencopy_manager_v1 as manager,
};

pub struct Screen {
    pub name: String,
    pub image: ColorImage,
}
struct Output {
    global: u32,
    proxy: wl_output::WlOutput,
    name: String,
    transform: u32,
}
struct Buffer {
    file: File,
    proxy: wl_buffer::WlBuffer,
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
}
#[derive(Default)]
struct State {
    outputs: Vec<Output>,
    shm: Option<wl_shm::WlShm>,
    manager: Option<manager::ZwlrScreencopyManagerV1>,
    synced: bool,
    buffer: Option<Buffer>,
    done: bool,
    error: Option<String>,
    inverted: bool,
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "zwlr_screencopy_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(2), qh, ()))
                }
                "wl_output" => state.outputs.push(Output {
                    global: name,
                    proxy: registry.bind(name, version.min(4), qh, ()),
                    name: String::new(),
                    transform: 0,
                }),
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => state.outputs.retain(|o| o.global != name),
            _ => {}
        }
    }
}
impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.synced = true;
    }
}
impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Some(o) = state.outputs.iter_mut().find(|o| o.proxy == *output) {
            match event {
                wl_output::Event::Name { name } => o.name = name,
                wl_output::Event::Geometry {
                    transform: WEnum::Value(t),
                    ..
                } => o.transform = t as u32,
                _ => {}
            }
        }
    }
}
impl Dispatch<frame::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        frame: &frame::ZwlrScreencopyFrameV1,
        event: frame::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            frame::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                let create = || -> Result<Buffer> {
                    ensure!(
                        width > 0 && height > 0 && width <= 16384 && height <= 16384,
                        "Invalid capture size"
                    );
                    let bytes = (stride as u64)
                        .checked_mul(height as u64)
                        .context("Capture buffer overflow")?;
                    ensure!(
                        bytes <= 256 * 1024 * 1024 && stride >= width * 4,
                        "Invalid capture stride"
                    );
                    ensure!(
                        matches!(
                            format,
                            wl_shm::Format::Argb8888
                                | wl_shm::Format::Xrgb8888
                                | wl_shm::Format::Abgr8888
                                | wl_shm::Format::Xbgr8888
                                | wl_shm::Format::Bgra8888
                                | wl_shm::Format::Bgrx8888
                                | wl_shm::Format::Rgba8888
                                | wl_shm::Format::Rgbx8888
                                | wl_shm::Format::Argb2101010
                                | wl_shm::Format::Xrgb2101010
                                | wl_shm::Format::Abgr2101010
                                | wl_shm::Format::Xbgr2101010
                        ),
                        "Unsupported screenshot pixel format {format:?}"
                    );
                    let file = tempfile::tempfile()?;
                    file.set_len(bytes)?;
                    let pool = state
                        .shm
                        .as_ref()
                        .context("Wayland shared memory unavailable")?
                        .create_pool(file.as_fd(), bytes as i32, qh, ());
                    let buffer = pool.create_buffer(
                        0,
                        width as i32,
                        height as i32,
                        stride as i32,
                        format,
                        qh,
                        (),
                    );
                    pool.destroy();
                    frame.copy(&buffer);
                    Ok(Buffer {
                        file,
                        proxy: buffer,
                        width,
                        height,
                        stride,
                        format,
                    })
                };
                match create() {
                    Ok(buffer) => state.buffer = Some(buffer),
                    Err(e) => {
                        state.error = Some(format!("{e:#}"));
                        state.done = true;
                    }
                }
            }
            frame::Event::Flags {
                flags: WEnum::Value(flags),
            } => state.inverted = flags.contains(frame::Flags::YInvert),
            frame::Event::Ready { .. } => state.done = true,
            frame::Event::Failed => {
                state.done = true;
                state.error =
                    Some("Screen capture unavailable (output may be powered off).".into());
            }
            _ => {}
        }
    }
}
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore manager::ZwlrScreencopyManagerV1);

pub struct Capturer {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
}
impl Capturer {
    pub fn connect() -> Result<Self> {
        Self::new(Connection::connect_to_env()?)
    }
    pub fn at(socket: &Path) -> Result<Self> {
        Self::new(Connection::from_socket(UnixStream::connect(socket)?)?)
    }
    fn new(connection: Connection) -> Result<Self> {
        let queue = connection.new_event_queue();
        let state = State::default();
        connection.display().get_registry(&queue.handle(), ());
        let mut capturer = Self {
            connection,
            queue,
            state,
        };
        capturer.sync()?;
        capturer.sync()?;
        ensure!(
            capturer.state.manager.is_some(),
            "Sway screencopy protocol unavailable"
        );
        Ok(capturer)
    }
    fn wait(&mut self, complete: impl Fn(&State) -> bool) -> Result<()> {
        let start = Instant::now();
        loop {
            self.queue.dispatch_pending(&mut self.state)?;
            if complete(&self.state) {
                return Ok(());
            }
            ensure!(
                start.elapsed() < Duration::from_secs(2),
                "Screen capture timed out"
            );
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            self.connection.flush()?;
            let backend = self.connection.backend();
            let fd = backend.poll_fd();
            let mut fds = [rustix::event::PollFd::new(
                &fd,
                rustix::event::PollFlags::IN,
            )];
            let remaining = Duration::from_secs(2).saturating_sub(start.elapsed());
            let timeout = rustix::event::Timespec {
                tv_sec: remaining.as_secs() as i64,
                tv_nsec: remaining.subsec_nanos() as i64,
            };
            let ready = rustix::event::poll(&mut fds, Some(&timeout))?;
            if ready > 0 {
                guard.read()?;
            } else {
                drop(guard);
                bail!("Screen capture timed out");
            }
        }
    }
    fn sync(&mut self) -> Result<()> {
        self.state.synced = false;
        self.connection.display().sync(&self.queue.handle(), ());
        self.wait(|s| s.synced)
    }
    pub fn screens(&mut self) -> Result<Vec<Screen>> {
        self.sync()?; // Process output hotplug/removal and transforms before each capture.
        if self.state.outputs.iter().any(|o| o.name.is_empty()) {
            // Newly bound outputs deliver their names after the registry event.
            self.sync()?;
        }
        let outputs: Vec<_> = self
            .state
            .outputs
            .iter()
            .filter(|o| !o.name.is_empty())
            .map(|o| (o.name.clone(), o.proxy.clone(), o.transform))
            .collect();
        let mut screens = Vec::new();
        let mut errors = Vec::new();
        for (name, output, transform) in outputs {
            self.state.done = false;
            self.state.error = None;
            self.state.inverted = false;
            self.state.buffer = None;
            let frame = self.state.manager.as_ref().unwrap().capture_output(
                0,
                &output,
                &self.queue.handle(),
                (),
            );
            let result = self.wait(|s| s.done);
            frame.destroy();
            let buffer = self.state.buffer.take();
            match result.and_then(|_| {
                if let Some(e) = self.state.error.take() {
                    bail!("{e}");
                }
                let buffer = buffer.as_ref().context("No screen buffer")?;
                let mut bytes = vec![0; buffer.stride as usize * buffer.height as usize];
                buffer.file.read_exact_at(&mut bytes, 0)?;
                thumbnail(
                    &bytes,
                    buffer.width,
                    buffer.height,
                    buffer.stride,
                    buffer.format,
                    self.state.inverted,
                    transform,
                )
            }) {
                Ok(image) => screens.push(Screen { name, image }),
                Err(e) => errors.push(format!("{name}: {e:#}")),
            }
            if let Some(buffer) = buffer {
                buffer.proxy.destroy();
            }
        }
        ensure!(
            !screens.is_empty(),
            "{}",
            if errors.is_empty() {
                "No active screens available".into()
            } else {
                errors.join("; ")
            }
        );
        Ok(screens)
    }
}

/// Convert native-endian SHM pixels, account for output transform/row inversion,
/// and downsample on the worker so the GUI never uploads full-resolution frames.
#[allow(clippy::too_many_arguments)]
pub fn thumbnail(
    data: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
    inverted: bool,
    transform: u32,
) -> Result<ColorImage> {
    ensure!(
        width > 0
            && height > 0
            && stride >= width * 4
            && data.len() >= stride as usize * height as usize,
        "Invalid screen buffer"
    );
    let swapped = transform % 2 == 1;
    let (tw, th) = if swapped {
        (height, width)
    } else {
        (width, height)
    };
    let ratio = (640. / tw as f32).min(450. / th as f32).min(1.);
    let (dw, dh) = (
        (tw as f32 * ratio).round().max(1.) as usize,
        (th as f32 * ratio).round().max(1.) as usize,
    );
    let mut pixels = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        for x in 0..dw {
            let ox = ((x as f32 + 0.5) * tw as f32 / dw as f32).floor() as u32;
            let oy = ((y as f32 + 0.5) * th as f32 / dh as f32).floor() as u32;
            let (mut sx, mut sy) = match transform % 4 {
                0 => (ox, oy),
                1 => (oy, height - 1 - ox),
                2 => (width - 1 - ox, height - 1 - oy),
                _ => (width - 1 - oy, ox),
            };
            // wl_output flipped transforms reflect the logical X axis after rotation.
            if transform >= 4 {
                let reflected_x = tw - 1 - ox;
                (sx, sy) = match transform % 4 {
                    0 => (reflected_x, oy),
                    1 => (oy, height - 1 - reflected_x),
                    2 => (width - 1 - reflected_x, height - 1 - oy),
                    _ => (width - 1 - oy, reflected_x),
                };
            }
            if inverted {
                sy = height - 1 - sy;
            }
            let offset = sy as usize * stride as usize + sx as usize * 4;
            let value = u32::from_ne_bytes(data[offset..offset + 4].try_into()?);
            let rgb = match format {
                wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => {
                    [(value >> 16) as u8, (value >> 8) as u8, value as u8]
                }
                wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => {
                    [value as u8, (value >> 8) as u8, (value >> 16) as u8]
                }
                wl_shm::Format::Bgra8888 | wl_shm::Format::Bgrx8888 => {
                    [(value >> 8) as u8, (value >> 16) as u8, (value >> 24) as u8]
                }
                wl_shm::Format::Rgba8888 | wl_shm::Format::Rgbx8888 => {
                    [(value >> 24) as u8, (value >> 16) as u8, (value >> 8) as u8]
                }
                wl_shm::Format::Argb2101010 | wl_shm::Format::Xrgb2101010 => [
                    (((value >> 20) & 1023) * 255 / 1023) as u8,
                    (((value >> 10) & 1023) * 255 / 1023) as u8,
                    ((value & 1023) * 255 / 1023) as u8,
                ],
                wl_shm::Format::Abgr2101010 | wl_shm::Format::Xbgr2101010 => [
                    ((value & 1023) * 255 / 1023) as u8,
                    (((value >> 10) & 1023) * 255 / 1023) as u8,
                    (((value >> 20) & 1023) * 255 / 1023) as u8,
                ],
                _ => bail!("Unsupported screenshot format {format:?}"),
            };
            pixels.push(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
        }
    }
    Ok(ColorImage::new([dw, dh], pixels))
}
pub fn poll(ctx: eframe::egui::Context) -> mpsc::Receiver<Result<Vec<Screen>>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut capturer = None;
        loop {
            let result = (|| {
                if capturer.is_none() {
                    capturer = Some(Capturer::connect()?);
                }
                capturer.as_mut().unwrap().screens()
            })();
            if result.is_err() {
                capturer = None;
            }
            match sender.try_send(result) {
                Ok(()) => ctx.request_repaint(),
                Err(mpsc::TrySendError::Disconnected(_)) => break,
                Err(mpsc::TrySendError::Full(_)) => {}
            }
            std::thread::sleep(Duration::from_millis(333));
        }
    });
    receiver
}
