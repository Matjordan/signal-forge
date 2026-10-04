mod app;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1440.0,900.0]).with_min_inner_size([900.0,600.0]),
        ..Default::default()
    };
    eframe::run_native("Signal Forge", options, Box::new(|cc| Ok(Box::new(app::Workbench::new(cc)))))
}
