mod app;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut initial_ports = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => match args.next() {
                Some(path) => initial_ports.push(path),
                None => { eprintln!("--port requires a device path"); std::process::exit(2); }
            },
            "--help" | "-h" => { println!("Usage: signal-forge [--port /dev/ttyUSB0] ..."); return Ok(()); }
            _ => { eprintln!("Unknown argument: {arg}"); std::process::exit(2); }
        }
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1440.0,900.0]).with_min_inner_size([900.0,600.0]),
        ..Default::default()
    };
    eframe::run_native("Signal Forge", options, Box::new(move |cc| Ok(Box::new(app::Workbench::new(cc,initial_ports)))))
}
