fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Achievement Hunter")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "achievement-hunter",
        options,
        Box::new(|cc| Ok(Box::new(achievement_hunter::app::App::new(cc)))),
    )
}
