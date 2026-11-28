fn main() {
    println!("cargo:rerun-if-changed=packaging/launcher.ico");
    println!("cargo:rerun-if-changed=packaging/windows-icon.rc");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("packaging/windows-icon.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed the Windows launcher icon");
    }
}
