// Generated decoders — included from build.rs output.
include!(concat!(env!("OUT_DIR"), "/riscv32_decode.rs"));

mod decode16_impl {
    // Re-import arg structs from parent (32-bit decoder).
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/riscv16_decode.rs"));
}

pub use decode16_impl::{decode16, Decode16};
