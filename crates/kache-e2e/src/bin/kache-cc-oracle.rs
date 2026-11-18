#[cfg(unix)]
fn main() {
    std::process::exit(kache_e2e::cc_oracle_cli::main(std::env::args_os().collect()));
}

#[cfg(not(unix))]
fn main() {
    eprintln!("kache-cc-oracle runs on Unix hosts only");
    std::process::exit(2);
}
