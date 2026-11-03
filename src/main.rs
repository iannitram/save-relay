//! `save-relay`: see `cli` for the commands.

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut stdout = std::io::stdout();
    let code = save_relay::cli::run(&args, &mut std::io::stdin(), &mut stdout);
    let _ = stdout.flush();
    std::process::exit(code);
}
