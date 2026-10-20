fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, rest @ ..] = args.as_slice()
        && flag == "--import-guide"
    {
        if let Err(error) = import_guide(rest) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }

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

/// `achievement-hunter --import-guide <game name|appid> <page file>`
///
/// Converts a saved/copied guide page into the per-game guide file the app
/// reads on startup, then slices it into each achievement.
fn import_guide(args: &[String]) -> Result<(), String> {
    let [game, path] = args else {
        return Err(
            "usage: achievement-hunter --import-guide <game name|appid> <page file>".to_owned(),
        );
    };
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let store = achievement_hunter::store::Store::load();
    if store.games.is_empty() {
        return Err("library is empty; start the app and load your games first".to_owned());
    }
    let (appid, title) = resolve_game(&store.games, game)?;
    let (input, output) = achievement_hunter::providers::save_guide_page(appid, &title, &raw)?;
    println!(
        "Saved \"{title}\" (appid {appid}): {input} -> {output} bytes ({:.1}% of the page)",
        output as f64 * 100.0 / input.max(1) as f64
    );
    println!("Guide text is picked up automatically on the game page after a restart.");
    Ok(())
}

fn resolve_game(
    games: &[achievement_hunter::model::Game],
    needle: &str,
) -> Result<(u32, String), String> {
    if let Ok(appid) = needle.parse::<u32>() {
        return match games.iter().find(|game| game.appid == appid) {
            Some(game) => Ok((game.appid, game.name.clone())),
            None => Err(format!("no game with appid {appid} in the library")),
        };
    }
    let needle = needle.trim().to_lowercase();
    let exact: Vec<_> = games
        .iter()
        .filter(|game| game.name.to_lowercase() == needle)
        .collect();
    if let [game] = exact.as_slice() {
        return Ok((game.appid, game.name.clone()));
    }
    let partial: Vec<_> = games
        .iter()
        .filter(|game| game.name.to_lowercase().contains(&needle))
        .collect();
    match partial.as_slice() {
        [game] => Ok((game.appid, game.name.clone())),
        [] => Err(format!("no game matching \"{needle}\" in the library")),
        many => {
            let names = many
                .iter()
                .take(10)
                .map(|game| format!("{} ({})", game.name, game.appid))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!("ambiguous game \"{needle}\"; try one of: {names}"))
        }
    }
}
