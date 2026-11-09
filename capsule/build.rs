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
    embed_setup_core();
}

/// 把 Inno 核心安装包复制到 OUT_DIR，供 setup_ui 内嵌，最终对外只发布一个安装器。
///
/// 核心不存在时写 0 字节占位，保证任何时刻 `cargo build` 都能通过；
/// setup_ui 在运行时发现内嵌为空会回退到同目录的 core 文件（调试用）。
fn embed_setup_core() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let core = manifest.join("installer/output/hetao-capsule-setup-core.exe");
    let out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("setup-core.bin");
    println!("cargo:rerun-if-changed={}", core.display());
    if core.is_file() {
        if std::fs::copy(&core, &out).is_err() {
            let _ = std::fs::write(&out, b"");
        }
    } else {
        let _ = std::fs::write(&out, b"");
    }
}
