fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "today".into());
    match cmd.as_str() {
        "today" | "week" | "top" => eprintln!("screentime {cmd}: not implemented yet"),
        "categorize" => {
            let target = std::env::args().nth(2).unwrap_or_default();
            let rules = screentime_core::Rules::builtin();
            let category = if target.ends_with(".desktop") {
                rules.categorize_app(&target)
            } else {
                rules.categorize_domain(&target)
            };
            println!("{category}");
        }
        _ => {
            eprintln!("usage: screentime [today|week|top|categorize <app_id|domain>]");
            std::process::exit(2);
        }
    }
}
