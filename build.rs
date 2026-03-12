fn main() {
    // XMoveWindow is called in popup.rs for X11 window positioning.
    println!("cargo:rustc-link-lib=X11");
}
