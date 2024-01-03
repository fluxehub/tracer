use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/shaders/shaders.hlsl");
    Command::new("C:\\Program Files (x86)\\Windows Kits\\10\\bin\\10.0.22621.0\\x64\\dxc.exe") // This is extreme laziness
        .args([
            "src/shaders/shaders.hlsl",
            "/T",
            "lib_6_3",
            "/Fo",
            "src/shaders/shaders.bin",
        ])
        .status()
        .unwrap();
}
