mod app;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut initial_ports = Vec::new();
    let mut update_enabled = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("Signal Forge {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--install-package" => {
                let root = args.next().unwrap_or_else(|| {
                    eprintln!("--install-package requires package root and prefix");
                    std::process::exit(2)
                });
                let prefix = args.next().unwrap_or_else(|| {
                    eprintln!("--install-package requires installation prefix");
                    std::process::exit(2)
                });
                if let Err(error) = signal_forge::desktop::install_package(
                    std::path::Path::new(&root),
                    std::path::Path::new(&prefix),
                ) {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
                return Ok(());
            }
            "--uninstall-prefix" => {
                let prefix = args.next().unwrap_or_else(|| {
                    eprintln!("--uninstall-prefix requires installation prefix");
                    std::process::exit(2)
                });
                if let Err(error) = signal_forge::desktop::uninstall(std::path::Path::new(&prefix))
                {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
                return Ok(());
            }
            "--no-update-check" => update_enabled = false,
            "--port" => match args.next() {
                Some(path) => initial_ports.push(path),
                None => {
                    eprintln!("--port requires a device path");
                    std::process::exit(2);
                }
            },
            "--help" | "-h" => {
                println!("Usage: signal-forge [--port /dev/ttyUSB0] ... [--no-update-check]");
                return Ok(());
            }
            _ => {
                eprintln!("Unknown argument: {arg}");
                std::process::exit(2);
            }
        }
    }
    if let Err(error) = signal_forge::desktop::repair_current_install() {
        log::warn!("Desktop integration could not be refreshed: {error}");
    }
    let update_failure = signal_forge::updater::take_restart_failure();
    let ready_signal = signal_forge::updater::take_restart_signal();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_app_id("signal-forge")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../assets/icons/signal-forge-256.png"
                ))
                .expect("Bundled Signal Forge icon"),
            ),
        ..Default::default()
    };
    let restart = std::sync::Arc::new(std::sync::Mutex::new(None));
    let restart_for_app = restart.clone();
    let result = eframe::run_native(
        "Signal Forge",
        options,
        Box::new(move |cc| {
            let workbench = app::Workbench::new(
                cc,
                initial_ports,
                update_enabled,
                restart_for_app,
                update_failure,
            );
            signal_forge::updater::signal_workbench_ready(ready_signal);
            Ok(Box::new(workbench))
        }),
    );
    // run_native drops Workbench first: save workspace, stop captures, release devices.
    if let Some(update) = restart.lock().unwrap().take() {
        if let Err(error) = update.restart() {
            eprintln!("{error}");
        }
    }
    result
}
