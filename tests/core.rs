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
    assert_eq!(
        patch
            .after
            .lines()
            .filter(|l| l.starts_with("include"))
            .count(),
        1
    );
    assert!(patch.after.lines().last().unwrap().starts_with("include"));
    let diff = patch.diff();
    // Reconstruct the changed section: context+removed is old, context+added is new.
    assert_eq!(
        diff.iter()
            .skip(3)
            .filter(|(k, _)| *k != '+')
            .map(|(_, l)| l.as_str())
            .collect::<Vec<_>>(),
        duplicate.lines().collect::<Vec<_>>()
    );
    patch.apply().unwrap();
    assert!(Patch::check(&config, &include).unwrap().installed());
}

#[test]
fn live_workspace_previews_use_actual_window_geometry_and_include_floating_windows() {
    let tree:layman::live::Node=serde_json::from_value(serde_json::json!({
        "type":"root","nodes":[{"type":"output","name":"DP-1","nodes":[
            {"type":"workspace","name":"1","rect":{"x":100,"y":20,"width":1000,"height":700},"nodes":[
                {"id":9,"type":"con","app_id":"terminal","focused":true,"rect":{"x":100,"y":20,"width":600,"height":700}}
            ],"floating_nodes":[{"type":"con","window":42,"name":"Dialog","rect":{"x":350,"y":200,"width":300,"height":150}}]},
            {"type":"workspace","name":"__i3_scratch"}
        ]}]
    })).unwrap();
    let workspaces = layman::live::workspaces(&tree);
    assert_eq!(workspaces.len(), 1);
    let w = &workspaces[0];
    assert_eq!(w.output, "DP-1");
    assert_eq!(w.rect.x, 100.);
    assert_eq!(w.windows.len(), 2);
    assert!(w.windows[0].focused);
    assert_eq!(w.windows[0].rect.width, 600.);
    assert!(w.windows[1].floating);
    assert_eq!(w.windows[1].title, "Dialog");
    for angle in [0, 90, 180, 270] {
        for flipped in [false, true] {
            assert!(
                layman::model::TRANSFORMS.contains(&layman::ui::transform(angle, flipped).as_str())
            );
        }
    }
}
