use std::path::PathBuf;
use std::process::Command;

/// Check whether the RISC-V cross-compiler is installed.
fn has_riscv_gcc() -> bool {
    Command::new("riscv64-linux-gnu-gcc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Workspace root (two levels up from CARGO_MANIFEST_DIR).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Build guest test programs via Makefile.
fn build_guest_programs() {
    let guest_dir = workspace_root().join("tests/guest");
    let status = Command::new("make")
        .arg("-C")
        .arg(&guest_dir)
        .status()
        .expect("failed to run make");
    assert!(status.success(), "make failed");
}

#[test]
fn guest_hello_world() {
    if !has_riscv_gcc() {
        eprintln!(
            "SKIP: riscv64-linux-gnu-gcc not found, \
             install with: apt install gcc-riscv64-linux-gnu"
        );
        return;
    }

    build_guest_programs();

    let bin = env!("CARGO_BIN_EXE_tcg-riscv64");
    let elf = workspace_root().join("tests/guest/build/riscv64/hello");

    let output = Command::new(bin)
        .arg(&elf)
        .output()
        .expect("failed to run tcg-riscv64");

    assert!(
        output.status.success(),
        "tcg-riscv64 exited with {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hello, World!\n",);
}
