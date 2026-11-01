//! Says its arguments with the platform's narrator, as the game would:
//! `cargo run -p horizon-narrator --example say -- [--volume 0.5] Hello there`.
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut volume = 1.0;
    if args.first().is_some_and(|a| a == "--volume") && args.len() > 1 {
        volume = args[1].parse().unwrap_or(1.0);
        args.drain(..2);
    }
    horizon_narrator::init();
    horizon_narrator::set_status(horizon_narrator::Status::All);
    horizon_narrator::set_volume(volume);
    if !horizon_narrator::is_active() {
        eprintln!("The narrator is not available on this system");
        std::process::exit(1);
    }
    let message = if args.is_empty() {
        "Narrator test".to_owned()
    } else {
        args.join(" ")
    };
    horizon_narrator::say_system_queued(&message);
    // The speech thread speaks asynchronously; give it time to finish.
    std::thread::sleep(std::time::Duration::from_millis(
        400 + 90 * message.len() as u64,
    ));
    println!("said: {message}");
}
