use anyhow::{Result, bail};
use layman::{ipc, preview, store, ui};

fn main() {
    if let Err(e) = run() {
        if std::env::args_os().nth(1).is_some_and(|a| a == "--guard") {
            println!("{e:#}");
        }
        eprintln!("layman: {e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.first().and_then(|s| s.to_str()) {
        Some("--help" | "-h") => {
            println!(
                "Layman — native display layouts for Sway\n\n  layman              Open the editor\n  layman --demo       Explore with sample displays (no Sway or writes)\n  layman --inspect    Print current outputs/workspaces as JSON (read-only)\n  layman --restore    Apply the saved layout and existing workspace placement\n  layman --print-config   Print the include line to add to your Sway config\n\nLayouts are saved under $XDG_CONFIG_HOME/layman (default ~/.config/layman).\nPreview changes revert after 20 seconds unless confirmed."
            );
            return Ok(());
        }
        Some("--inspect") if args.len() == 1 => {
            println!("{}", serde_json::to_string_pretty(&ipc::snapshot()?)?);
            return Ok(());
        }
        Some("--restore") if args.len() == 1 => {
            if let Some(layout) = store::load()? {
                let Some(_lock) = store::mutation_lock()? else {
                    return Ok(());
                };
                ipc::apply(&layout)?;
            }
            return Ok(());
        }
        Some("--print-config") if args.len() == 1 => {
            println!("{}", store::include_line()?);
            return Ok(());
        }
        Some("--guard") if args.len() == 2 => {
            return preview::guard(std::path::Path::new(&args[1]));
        }
        Some("--demo") if args.len() == 1 => {}
        None => {}
        _ => bail!("Unknown arguments. Use --help."),
    }
    let demo = args.first().is_some_and(|a| a == "--demo");
    // Fail clearly before opening a window; never substitute fake monitors for IPC failure.
    if !demo {
        ipc::snapshot()?;
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180., 760.])
            .with_min_inner_size([900., 620.])
            .with_app_id("dev.layman.Layman"),
        renderer: eframe::Renderer::Glow,
        multisampling: 4,
        ..Default::default()
    };
    eframe::run_native(
        "Layman",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(&cc.egui_ctx, demo)?))),
    )
    .map_err(|e| anyhow::anyhow!("Cannot start native UI: {e}"))
}
