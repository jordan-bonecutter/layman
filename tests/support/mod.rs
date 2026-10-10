use anyhow::{Context, Result};
use std::{os::unix::net::UnixStream, path::Path, time::Duration};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    protocol::{wl_output, wl_pointer, wl_registry},
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
};

#[derive(Default)]
struct State {
    manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
    outputs: Vec<(wl_output::WlOutput, String)>,
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "zwlr_virtual_pointer_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(2), qh, ()))
                }
                "wl_output" => state
                    .outputs
                    .push((registry.bind(name, version.min(4), qh, ()), String::new())),
                _ => {}
            }
        }
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
        if let wl_output::Event::Name { name } = event
            && let Some(o) = state.outputs.iter_mut().find(|o| &o.0 == output)
        {
            o.1 = name;
        }
    }
}
delegate_noop!(State: ignore zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);

pub struct Pointer {
    connection: Connection,
    pointer: zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
}
impl Pointer {
    pub fn new(socket: &Path) -> Result<Self> {
        // Explicit socket path: no process environment changes, no desktop connection.
        let connection = Connection::from_socket(UnixStream::connect(socket)?)?;
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;
        let output = state
            .outputs
            .iter()
            .find(|o| o.1 == "HEADLESS-1")
            .context("HEADLESS-1 Wayland output")?;
        let pointer = state
            .manager
            .as_ref()
            .context("Virtual pointer manager")?
            .create_virtual_pointer_with_output(None, Some(&output.0), &qh, ());
        queue.roundtrip(&mut state)?;
        Ok(Self {
            connection,
            pointer,
        })
    }
    pub fn click(&self, x: u32, y: u32) -> Result<()> {
        self.motion(x, y)?;
        self.button(true)?;
        std::thread::sleep(Duration::from_millis(60));
        self.button(false)?;
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }
    pub fn motion(&self, x: u32, y: u32) -> Result<()> {
        self.pointer.motion_absolute(0, x, y, 1280, 800);
        self.pointer.frame();
        self.connection.flush()?;
        std::thread::sleep(Duration::from_millis(60));
        Ok(())
    }
    pub fn button(&self, pressed: bool) -> Result<()> {
        self.pointer.button(
            0,
            0x110,
            if pressed {
                wl_pointer::ButtonState::Pressed
            } else {
                wl_pointer::ButtonState::Released
            },
        );
        self.pointer.frame();
        self.connection.flush()?;
        Ok(())
    }
}
