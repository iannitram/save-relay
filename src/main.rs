//! `save-relay`: see `cli` for the commands.

use std::io::Write;

fn main() {
    let raw: Vec<_> = std::env::args_os().skip(1).collect();
    let mut stdout = std::io::stdout();
    let json = raw.iter().any(|arg| arg == "--json");
    let args: Vec<String> = match raw.into_iter().map(|arg| arg.into_string()).collect() {
        Ok(args) => args,
        Err(_) => {
            let code = save_relay::cli::non_utf8_argument(json, &mut stdout);
            let _ = stdout.flush();
            std::process::exit(code);
        }
    };
    let code = save_relay::cli::run(&args, &mut std::io::stdin(), &mut stdout);
    let _ = stdout.flush();
    std::process::exit(code);
}
