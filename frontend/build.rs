use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let decode_file = Path::new("decode/riscv32.decode");

    println!("cargo::rerun-if-changed={}", decode_file.display());

    let input =
        fs::read_to_string(decode_file).expect("failed to read riscv32.decode");

    let mut output = Vec::new();
    decodetree::generate(&input, &mut output)
        .expect("decodetree code generation failed");

    let out_path = Path::new(&out_dir).join("riscv32_decode.rs");
    fs::write(&out_path, output).expect("failed to write generated decoder");
}
