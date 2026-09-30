mod app;
mod db;
mod model;
mod player;
mod sync;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 640.0])
            .with_title("IPTV-Org Player"),
        ..Default::default()
    };
    eframe::run_native(
        "IPTV-Org Player",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
