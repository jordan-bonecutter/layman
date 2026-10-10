use layman::{
    ipc,
    model::{quote, snap},
    store, ui,
};
use std::{
    io::{Read, Write},
    os::unix::net::UnixListener,
};

#[test]
fn scale_rotation_and_adjacency_use_logical_pixels() {
    let mut l = ui::demo_layout();
    assert_eq!(l.outputs[1].size(), (1536, 960));
    assert!(l.warnings().is_empty());
    l.outputs[1].transform = "90".into();
    assert_eq!(l.outputs[1].size(), (960, 1536));
    l.horizontal();
    assert_eq!(l.outputs[1].rect.x, 2560);
    assert!(l.warnings().is_empty());
    assert_eq!(snap(&l, 1, 2557, 4, 10), (2560, 0));
    l.outputs[1].rect.x += 50;
    assert!(l.warnings().iter().any(|w| w.contains("do not touch")));
    l.outputs[1].rect.x = 2000;
    assert!(l.warnings().iter().any(|w| w.contains("overlap")));
}
#[test]
fn rejects_blank_screens_invalid_numbers_and_command_injection() {
    let mut l = ui::demo_layout();
    for o in &mut l.outputs {
        o.active = false;
    }
    assert!(l.validate().is_err());
    l = ui::demo_layout();
    l.outputs[0].scale = f64::NAN;
    assert!(l.validate().is_err());
    l = ui::demo_layout();
    l.outputs[0].transform = "normal; exit".into();
    assert!(l.validate().is_err());
    l = ui::demo_layout();
    l.workspaces[0].output = "unplugged".into();
    assert!(l.validate().is_err());
    for name in ["a; exit", "a,b", "a\nb", "a\"b", "$HOME", "`id`", "a\\b"] {
        assert!(quote(name).is_err(), "{name}");
    }
    assert_eq!(quote("1: code").unwrap(), "\"1: code\"");
}
#[test]
fn resolves_identity_reconnections_and_refuses_to_blank_undocked_screen() {
    let saved = ui::demo_layout();
    let mut live = saved.clone();
    live.outputs[0].name = "DP-9".into();
    let l = saved.connected(&live).unwrap();
    assert_eq!(l.outputs[0].name, "DP-9");
    assert_eq!(l.workspaces[0].output, "DP-9");
    let mut saved = saved;
    saved.outputs[1].active = false;
    saved.workspaces.retain(|w| w.output != "eDP-1");
    live.outputs.remove(0);
    assert!(saved.connected(&live).is_err());
}
#[test]
fn save_roundtrip_and_reload_include_are_complete() {
    let dir = tempfile::tempdir().unwrap();
    let l = ui::demo_layout();
    store::save_at(
        dir.path(),
        &l,
        std::path::Path::new("/opt/layman/bin/layman"),
    )
    .unwrap();
    let saved = store::load_at(dir.path()).unwrap().unwrap();
    assert_eq!(saved.outputs[0].current_mode, l.outputs[0].current_mode);
    let config = std::fs::read_to_string(dir.path().join("sway.conf")).unwrap();
    assert!(config.contains("exec_always \"/opt/layman/bin/layman\" --restore"));
    assert!(config.contains("workspace \"1: code\" output \"Dell Studio display demo-1\""));
    let mut disabled = l;
    disabled.outputs[1].active = false;
    disabled.workspaces.retain(|w| w.output != "eDP-1");
    assert!(
        !store::config(&disabled, std::path::Path::new("/opt/layman"))
            .unwrap()
            .contains(" disable")
    );
}
#[test]
fn ipc_reads_fragmented_frames_and_rejects_bad_headers() {
    for valid in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("ipc.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut header = [0; 14];
            stream.read_exact(&mut header).unwrap();
            assert_eq!(&header[..6], b"i3-ipc");
            assert_eq!(u32::from_le_bytes(header[10..14].try_into().unwrap()), 3);
            let body = b"[1,2,3]";
            let mut response = if valid {
                b"i3-ipc".to_vec()
            } else {
                b"badipc".to_vec()
            };
            response.extend((body.len() as u32).to_le_bytes());
            response.extend(3u32.to_le_bytes());
            if valid {
                response.extend(body);
            }
            for byte in response {
                stream.write_all(&[byte]).unwrap();
            }
        });
        let response = ipc::request_at::<Vec<i32>>(&socket, 3, "");
        if valid {
            assert_eq!(response.unwrap(), vec![1, 2, 3]);
        } else {
            assert!(response.is_err());
        }
        server.join().unwrap();
    }
}
#[test]
fn ui_renders_a_frame_without_a_display_server() {
    let ctx = eframe::egui::Context::default();
    let mut app = ui::App::new(&ctx, true).unwrap();
    let output = ctx.run(
        eframe::egui::RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(
                eframe::egui::Pos2::ZERO,
                eframe::egui::vec2(1180., 760.),
            )),
            ..Default::default()
        },
        |ctx| app.draw(ctx),
    );
    assert!(!output.shapes.is_empty());
    assert!(
        !ctx.tessellate(output.shapes, output.pixels_per_point)
            .is_empty()
    );
}

#[test]
fn config_patch_is_reviewable_idempotent_and_refuses_stale_edits() {
    use layman::integration::Patch;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let include = dir.path().join("layman/sway.conf");
    let before = "set $mod Mod4\noutput DP-1 scale 1\n";
    std::fs::write(&config, before).unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o640)).unwrap();
    let patch = Patch::check(&config, &include).unwrap();
    assert!(!patch.installed());
    assert!(
        patch
            .diff()
            .iter()
            .any(|(kind, line)| *kind == '+' && line.starts_with("include "))
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), before); // Checking never writes.
    patch.apply().unwrap();
    assert!(Patch::check(&config, &include).unwrap().installed());
    assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("config.layman-backup-")
        })
        .unwrap();
    assert_eq!(std::fs::read_to_string(backup.path()).unwrap(), before);
    std::fs::write(&config, before).unwrap();
    let patch = Patch::check(&config, &include).unwrap();
    std::fs::write(&config, "# edited since preview\n").unwrap();
    assert!(patch.apply().is_err());
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "# edited since preview\n"
    );
    let commented = format!("include \"{}\" # keep layouts\n", include.display());
    std::fs::write(&config, &commented).unwrap();
    assert!(Patch::check(&config, &include).unwrap().installed());
    let relative = "include layman/sway.conf\n";
    std::fs::write(&config, relative).unwrap();
    assert!(Patch::check(&config, &include).unwrap().installed());
    let duplicate = format!(
        "include \"{}\"\noutput DP-1 scale 2\ninclude {}\noutput DP-1 pos 0 0\n",
        include.display(),
        include.display()
    );
    std::fs::write(&config, &duplicate).unwrap();
    let patch = Patch::check(&config, &include).unwrap();
    assert!(patch.installed());
    assert_eq!(patch.after, duplicate);
    assert!(patch.diff().is_empty());
    assert!(patch.apply().is_err());
    // An existing early include is also left byte-for-byte intact.
    let early = format!(
        "include \"{}\" # layout\noutput DP-1 scale 2\n",
        include.display()
    );
    std::fs::write(&config, &early).unwrap();
    let patch = Patch::check(&config, &include).unwrap();
    assert!(patch.installed());
    assert_eq!(patch.after, early);
    // An append diff contains additions only, even if the original lacks a newline.
    std::fs::write(&config, "set $mod Mod4").unwrap();
    let patch = Patch::check(&config, &include).unwrap();
    assert!(!patch.diff().iter().any(|(kind, _)| *kind == '-'));
    assert!(patch.after.starts_with("set $mod Mod4\n"));
}

#[test]
fn screen_pixels_handle_stride_formats_rotation_mirroring_and_inversion() {
    use eframe::egui::Color32;
    use layman::capture::decode_frame;
    use wayland_client::protocol::wl_shm::Format;
    // A non-square, asymmetric image with padding at the end of each row.
    let data: Vec<u8> = [1_u32, 2, 3, 999, 4, 5, 6, 999]
        .into_iter()
        .flat_map(u32::to_ne_bytes)
        .collect();
    let expected = [
        vec![1, 2, 3, 4, 5, 6],
        vec![4, 1, 5, 2, 6, 3],
        vec![6, 5, 4, 3, 2, 1],
        vec![3, 6, 2, 5, 1, 4],
        vec![3, 2, 1, 6, 5, 4],
        vec![1, 4, 2, 5, 3, 6],
        vec![4, 5, 6, 1, 2, 3],
        vec![6, 3, 5, 2, 4, 1],
    ];
    for (transform, expected) in expected.iter().enumerate() {
        let image =
            decode_frame(&data, 3, 2, 16, Format::Xrgb8888, false, transform as u32).unwrap();
        assert_eq!(image.size, if transform % 2 == 0 { [3, 2] } else { [2, 3] });
        assert_eq!(
            image.pixels.iter().map(|p| p.b()).collect::<Vec<_>>(),
            *expected
        );
    }
    let inverted = decode_frame(&data, 3, 2, 16, Format::Xrgb8888, true, 0).unwrap();
    assert_eq!(
        inverted.pixels.iter().map(|p| p.b()).collect::<Vec<_>>(),
        [4, 5, 6, 1, 2, 3]
    );
    for (format, value) in [
        (Format::Xrgb8888, 0xff123456_u32),
        (Format::Xbgr8888, 0xff563412),
        (Format::Bgrx8888, 0x563412ff),
        (Format::Rgbx8888, 0x123456ff),
    ] {
        let image = decode_frame(&value.to_ne_bytes(), 1, 1, 4, format, false, 0).unwrap();
        assert_eq!(image.pixels[0], Color32::from_rgb(0x12, 0x34, 0x56));
    }
    let image = decode_frame(
        &0x3ff00000_u32.to_ne_bytes(),
        1,
        1,
        4,
        Format::Xrgb2101010,
        false,
        0,
    )
    .unwrap();
    assert_eq!(image.pixels[0], Color32::RED);
    assert!(decode_frame(&data, 3, 2, 8, Format::Xrgb8888, false, 0).is_err());
    assert!(decode_frame(&data[..4], 3, 2, 16, Format::Xrgb8888, false, 0).is_err());
}
