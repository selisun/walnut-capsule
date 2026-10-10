//! Windows 资源（应用图标）链接。
//!
//! 工具链为 x86_64-pc-windows-gnu 且环境无 windres/rc.exe，
//! 因此 assets/icon.syso 由 rsrc 工具预生成（单 .rsrc 段 COFF 目标，无重定位），
//! 这里直接作为链接对象传给链接器；lld 会识别 .rsrc 段并填充 PE 资源目录。

fn main() {
    let syso = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon.syso");
    if syso.exists() && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg={}", syso.display());
    }
    println!("cargo:rerun-if-changed=assets/icon.syso");
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
