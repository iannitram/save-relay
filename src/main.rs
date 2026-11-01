//! `save-relay`: see `cli` for the commands.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = save_relay::cli::run(&args, &mut std::io::stdin(), &mut std::io::stdout());
    std::process::exit(code);
}
