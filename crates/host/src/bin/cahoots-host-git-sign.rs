//! The program git runs as `gpg.ssh.program`. See `cahoots_host::gitsign`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cahoots_host::gitsign::main(&args));
}
