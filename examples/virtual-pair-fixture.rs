//! Process fixture for external-program PTY compatibility tests.
use signal_forge::virtual_pair::VirtualPair;
use std::io::{self, BufRead, Write};
fn main() {
    let directory = std::env::args().nth(1).expect("link directory");
    let pair = VirtualPair::create("fixture", Some(std::path::Path::new(&directory))).unwrap();
    println!(
        "{}",
        serde_json::json!({"paths":pair.paths,"raw_paths":pair.raw_paths})
    );
    io::stdout().flush().unwrap();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        if let Some(side) = line.strip_prefix("reset ") {
            let side: usize = side.parse().unwrap();
            println!(
                "{}",
                serde_json::json!({"released":pair.release_exclusive(side).is_ok()})
            );
            io::stdout().flush().unwrap();
        } else {
            break;
        }
    }
    drop(pair);
}
