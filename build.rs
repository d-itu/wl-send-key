use std::{env, fs, path::PathBuf};

use wayland_oxide_scanner::{Scanner, Side};

fn main() {
    let code = Scanner::new()
        .generate(
            [
                env::var_os("WAYLAND_XML").unwrap(),
                env::var_os("EXT_TRANSIENT_SEAT_V1_XML").unwrap(),
                env::var_os("VIRTUAL_KEYBOARD_XML").unwrap(),
            ],
            Side::Client,
        )
        .unwrap();

    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("wayland.rs"), code).unwrap();
}
