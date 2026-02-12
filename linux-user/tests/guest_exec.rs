use std::path::PathBuf;
use std::process::{Command, Output};

/// Guest test case definition.
struct GuestTest {
    name: &'static str,
    elf: &'static str,
    expected_stdout: &'static str,
}

const GUEST_TESTS: &[GuestTest] = &[
    GuestTest {
        name: "hello",
        elf: "hello",
        expected_stdout: "Hello, World!\n",
    },
    GuestTest {
        name: "hello_printf",
        elf: "hello_printf",
        expected_stdout: "Hello, World!\n",
    },
    GuestTest {
        name: "hello_float",
        elf: "hello_float",
        expected_stdout: "a=1.50 b=2.25 c=3.875000 \
            d=1.291667 f=3.875 i=3 u=4\n",
    },
];

fn has_riscv_gcc() -> bool {
    Command::new("riscv64-linux-gnu-gcc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn build_guest_programs() {
    let guest_dir = workspace_root().join("tests/guest");
    let status = Command::new("make")
        .arg("-C")
        .arg(&guest_dir)
        .status()
        .expect("failed to run make");
    assert!(status.success(), "make failed");
}

fn run_guest(elf_name: &str) -> Output {
    let bin = env!("CARGO_BIN_EXE_tcg-riscv64");
    let elf = workspace_root()
        .join("target/guest/riscv64")
        .join(elf_name);
    Command::new(bin)
        .arg(&elf)
        .output()
        .unwrap_or_else(|e| {
            panic!("failed to run tcg-riscv64 {}: {e}", elf.display())
        })
}

fn assert_guest(test: &GuestTest) {
    let out = run_guest(test.elf);
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "{}: exit {code}\nstdout: {stdout}\nstderr: {stderr}",
        test.name,
    );
    assert_eq!(
        stdout, test.expected_stdout,
        "{}: stdout mismatch\nstderr: {stderr}",
        test.name,
    );
}

fn ensure_built() {
    if !has_riscv_gcc() {
        panic!(
            "riscv64-linux-gnu-gcc not found, \
             install with: apt install gcc-riscv64-linux-gnu"
        );
    }
    build_guest_programs();
}

#[test]
fn guest_hello() {
    ensure_built();
    assert_guest(&GUEST_TESTS[0]);
}

#[test]
fn guest_hello_printf() {
    ensure_built();
    assert_guest(&GUEST_TESTS[1]);
}

#[test]
fn guest_hello_float() {
    ensure_built();
    assert_guest(&GUEST_TESTS[2]);
}

#[test]
fn guest_summary() {
    if !has_riscv_gcc() {
        eprintln!("SKIP: riscv64-linux-gnu-gcc not found");
        return;
    }
    build_guest_programs();

    let mut passed = 0u32;
    let mut failed = 0u32;

    eprintln!("\n=== Guest Execution Summary ===");
    for t in GUEST_TESTS {
        let out = run_guest(t.elf);
        let code = out.status.code().unwrap_or(-1);
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout == t.expected_stdout {
                eprintln!("  {:<20} PASS", t.name);
                passed += 1;
            } else {
                eprintln!(
                    "  {:<20} FAIL (exit {code})",
                    t.name
                );
                eprintln!(
                    "    expected: {:?}",
                    t.expected_stdout
                );
                eprintln!("    got:      {stdout:?}");
                failed += 1;
            }
        } else {
            let stderr =
                String::from_utf8_lossy(&out.stderr);
            eprintln!(
                "  {:<20} FAIL (exit {code})",
                t.name
            );
            if !stderr.is_empty() {
                eprintln!("    stderr: {stderr}");
            }
            failed += 1;
        }
    }
    eprintln!("---");
    eprintln!(
        "Result: {passed} passed, {failed} failed"
    );
    assert_eq!(failed, 0, "{failed} guest test(s) failed");
}