#[cfg(windows)]
fn windows_version(version: &str) -> String {
    let mut parts: Vec<u32> = version
        .split('.')
        .take(4)
        .map(|part| {
            part.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect();
    while parts.len() < 4 {
        parts.push(0);
    }
    parts
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn main() {
    println!("cargo:rerun-if-changed=assets/mirrorswitch.ico");

    #[cfg(windows)]
    {
        // Build scripts compile for the host. When cross-checking a macOS
        // target from Windows, do not try to embed Windows resources.
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
            return;
        }
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/mirrorswitch.ico");
        resource.set("ProductName", "MirrorSwitch");
        resource.set("FileDescription", "MirrorSwitch · Rust 镜像管家");
        resource.set("CompanyName", "MirrorSwitch contributors");
        resource.set(
            "LegalCopyright",
            "Copyright (c) 2026 MirrorSwitch contributors",
        );
        resource.set("OriginalFilename", "MirrorSwitch.exe");
        let version = windows_version(env!("CARGO_PKG_VERSION"));
        resource.set("ProductVersion", &version);
        resource.set("FileVersion", &version);
        if let Err(error) = resource.compile() {
            println!("cargo:warning=unable to embed Windows resources: {error}");
        }
    }
}
