mod elf;
mod guest_space;
mod loader;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

/// Guest test case definition.
struct GuestTest {
    name: &'static str,
    elf: &'static str,
    args: &'static [&'static str],
    expected_stdout: StdoutExpectation,
}

enum StdoutExpectation {
    Exact(&'static str),
    Contains(&'static [&'static str]),
}

const GUEST_TESTS: &[GuestTest] = &[
    GuestTest {
        name: "hello",
        elf: "hello",
        args: &[],
        expected_stdout: StdoutExpectation::Exact("Hello, World!\n"),
    },
    GuestTest {
        name: "hello_printf",
        elf: "hello_printf",
        args: &[],
        expected_stdout: StdoutExpectation::Exact("Hello, World!\n"),
    },
    GuestTest {
        name: "hello_float",
        elf: "hello_float",
        args: &[],
        expected_stdout: StdoutExpectation::Exact(
            "a=1.50 b=2.25 c=3.875000 \
            d=1.291667 f=3.875 i=3 u=4\n",
        ),
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
    let out = Command::new("make")
        .arg("-C")
        .arg(&guest_dir)
        .arg("--no-print-directory")
        .output()
        .expect("failed to run make");
    if !out.status.success() {
        let code = out.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        panic!(
            "make failed (exit {code})\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

fn runner_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let cargo =
            std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let mut cmd = Command::new(cargo);
        cmd.arg("build")
            .arg("-p")
            .arg("tcg-linux-user")
            .arg("--bin")
            .arg("tcg-riscv64")
            .arg("--quiet")
            .current_dir(workspace_root());
        if !cfg!(debug_assertions) {
            cmd.arg("--release");
        }
        let status = cmd.status().expect("failed to build tcg-riscv64");
        assert!(status.success(), "cargo build tcg-riscv64 failed");

        let profile =
            if cfg!(debug_assertions) { "debug" } else { "release" };
        let mut bin = workspace_root()
            .join("target")
            .join(profile)
            .join("tcg-riscv64");
        if cfg!(windows) {
            bin.set_extension("exe");
        }
        bin
    })
    .as_path()
}

fn run_guest(elf_name: &str, args: &[&str]) -> Output {
    let elf = workspace_root().join("target/guest/riscv64").join(elf_name);
    Command::new(runner_bin())
        .arg(&elf)
        .args(args)
        .output()
        .unwrap_or_else(|e| {
            panic!("failed to run tcg-riscv64 {}: {e}", elf.display())
        })
}

fn verify_stdout(test: &GuestTest, stdout: &str) -> Result<(), String> {
    match test.expected_stdout {
        StdoutExpectation::Exact(expected) => {
            if stdout == expected {
                Ok(())
            } else {
                Err(format!("expected: {expected:?}\n    got:      {stdout:?}"))
            }
        }
        StdoutExpectation::Contains(markers) => {
            let missing: Vec<&str> = markers
                .iter()
                .copied()
                .filter(|m| !stdout.contains(m))
                .collect();
            if missing.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "missing markers: {missing:?}\n    got:      {stdout:?}",
                ))
            }
        }
    }
}

fn assert_guest(test: &GuestTest) {
    let out = run_guest(test.elf, test.args);
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "{}: exit {code}\nstdout: {stdout}\nstderr: {stderr}",
        test.name,
    );
    if let Err(msg) = verify_stdout(test, &stdout) {
        panic!(
            "{}: stdout mismatch\n{}\nstderr: {}",
            test.name, msg, stderr
        );
    }
}

fn ensure_built() {
    if !has_riscv_gcc() {
        panic!(
            "riscv64-linux-gnu-gcc not found, \
             install with: apt install gcc-riscv64-linux-gnu"
        );
    }
    static BUILD_ONCE: OnceLock<()> = OnceLock::new();
    BUILD_ONCE.get_or_init(|| {
        build_guest_programs();
        let _ = runner_bin();
    });
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
    ensure_built();

    let mut passed = 0u32;
    let mut failed = 0u32;

    eprintln!("\n=== Guest Execution Summary ===");
    for t in GUEST_TESTS {
        let out = run_guest(t.elf, t.args);
        let code = out.status.code().unwrap_or(-1);
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Err(msg) = verify_stdout(t, &stdout) {
                eprintln!("  {:<20} FAIL (exit {code})", t.name);
                eprintln!("    {msg}");
                failed += 1;
            } else {
                eprintln!("  {:<20} PASS", t.name);
                passed += 1;
            }
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            eprintln!("  {:<20} FAIL (exit {code})", t.name);
            if !stderr.is_empty() {
                eprintln!("    stderr: {stderr}");
            }
            failed += 1;
        }
    }
    eprintln!("---");
    eprintln!("Result: {passed} passed, {failed} failed");
    assert_eq!(failed, 0, "{failed} guest test(s) failed");
}
