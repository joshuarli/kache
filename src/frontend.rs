mod version;

#[cfg(target_os = "macos")]
#[used]
#[unsafe(link_section = "__TEXT,__info_plist")]
static MACOS_INFO_PLIST: [u8; include_bytes!("Info.plist").len()] =
    *include_bytes!("Info.plist");

fn main() {
    let exit = match kache_client::run(version::VERSION) {
        Ok(exit) => exit,
        Err(error) => {
            eprintln!("error: {error:#}");
            1
        }
    };
    std::process::exit(exit);
}
