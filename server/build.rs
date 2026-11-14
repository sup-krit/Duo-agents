// Icon and version info in agent-duo.exe (version from Cargo.toml).
fn main() {
    println!("cargo:rerun-if-changed=assets/agent-duo.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/agent-duo.ico").set("ProductName", "Agent Duo").set("FileDescription", "Agent Duo").set("LegalCopyright", "");
        if let Err(e) = res.compile_for("agent-duo") {
            // no windres / rc.exe on this machine: the exe still works, just without an icon
            println!("cargo:warning=no icon embedded: {e}");
        }
    }
}
