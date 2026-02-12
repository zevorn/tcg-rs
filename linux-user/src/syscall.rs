use crate::guest_space::GuestSpace;

// RISC-V Linux syscall numbers
const SYS_IOCTL: u64 = 29;
const SYS_CLOSE: u64 = 57;
const SYS_WRITE: u64 = 64;
const SYS_WRITEV: u64 = 66;
const SYS_READLINKAT: u64 = 78;
const SYS_FSTAT: u64 = 80;
const SYS_EXIT: u64 = 93;
const SYS_EXIT_GROUP: u64 = 94;
const SYS_SET_TID_ADDRESS: u64 = 96;
const SYS_FUTEX: u64 = 98;
const SYS_SET_ROBUST_LIST: u64 = 99;
const SYS_CLOCK_GETTIME: u64 = 113;
const SYS_TGKILL: u64 = 131;
const SYS_RT_SIGACTION: u64 = 134;
const SYS_RT_SIGPROCMASK: u64 = 135;
const SYS_UNAME: u64 = 160;
const SYS_GETPID: u64 = 172;
const SYS_GETTID: u64 = 178;
const SYS_BRK: u64 = 214;
const SYS_MUNMAP: u64 = 215;
const SYS_MMAP: u64 = 222;
const SYS_MPROTECT: u64 = 226;
const SYS_MADVISE: u64 = 233;
const SYS_RISCV_HWPROBE: u64 = 258;
const SYS_PRLIMIT64: u64 = 261;
const SYS_GETRANDOM: u64 = 278;
const SYS_RSEQ: u64 = 293;

const ENOSYS: u64 = (-38i64) as u64;
const ENOTTY: u64 = (-25i64) as u64;
const ENOENT: u64 = (-2i64) as u64;

/// Syscall dispatch result.
pub enum SyscallResult {
    /// Continue execution (return value in a0).
    Continue(u64),
    /// Program exited with given code.
    Exit(i32),
}

/// Handle a RISC-V Linux syscall.
///
/// `regs` is the full GPR array (x0-x31).
/// Syscall number in a7 (x17), args in a0-a5 (x10-x15).
pub fn handle_syscall(
    space: &mut GuestSpace,
    regs: &mut [u64; 32],
    mmap_next: &mut u64,
    elf_path: &str,
) -> SyscallResult {
    let nr = regs[17]; // a7
    let a0 = regs[10];
    let a1 = regs[11];
    let a2 = regs[12];
    let a3 = regs[13];
    #[allow(unused_variables)]
    let a4 = regs[14];

    match nr {
        SYS_WRITE => {
            let fd = a0 as i32;
            let buf = a1;
            let len = a2 as usize;
            let host_buf = space.g2h(buf);
            let ret = unsafe {
                libc::write(fd, host_buf as *const libc::c_void, len)
            };
            if ret < 0 {
                let e = unsafe { *libc::__errno_location() };
                SyscallResult::Continue((-e) as u64)
            } else {
                SyscallResult::Continue(ret as u64)
            }
        }
        SYS_EXIT | SYS_EXIT_GROUP => SyscallResult::Exit(a0 as i32),
        SYS_BRK => {
            if a0 == 0 {
                SyscallResult::Continue(space.brk())
            } else if a0 >= space.brk() {
                let old = space.brk();
                let new_brk = crate::guest_space::page_align_up(a0);
                let old_aligned = crate::guest_space::page_align_up(old);
                if new_brk > old_aligned {
                    let sz = (new_brk - old_aligned) as usize;
                    let _ = space.mmap_fixed(
                        old_aligned,
                        sz,
                        libc::PROT_READ | libc::PROT_WRITE,
                    );
                }
                space.set_brk(a0);
                SyscallResult::Continue(a0)
            } else {
                SyscallResult::Continue(space.brk())
            }
        }
        SYS_MMAP => {
            let addr = a0;
            let len = a1 as usize;
            let prot = a2 as i32;
            let aligned_len =
                crate::guest_space::page_align_up(len as u64) as usize;
            let guest_addr = if addr != 0 {
                addr
            } else {
                let a = *mmap_next;
                *mmap_next += aligned_len as u64;
                a
            };
            match space.mmap_fixed(guest_addr, aligned_len, prot) {
                Ok(()) => SyscallResult::Continue(guest_addr),
                Err(_) => SyscallResult::Continue(
                    (-12i64) as u64, // ENOMEM
                ),
            }
        }
        SYS_MPROTECT => {
            let addr = a0;
            let len = a1 as usize;
            let prot = a2 as i32;
            match space.mprotect(addr, len, prot) {
                Ok(()) => SyscallResult::Continue(0),
                Err(_) => SyscallResult::Continue((-22i64) as u64),
            }
        }
        // Stubs that return success
        SYS_MUNMAP | SYS_SET_ROBUST_LIST | SYS_RT_SIGACTION
        | SYS_RT_SIGPROCMASK | SYS_MADVISE | SYS_CLOSE => {
            SyscallResult::Continue(0)
        }
        SYS_SET_TID_ADDRESS => {
            SyscallResult::Continue(1) // fake TID
        }
        SYS_GETPID | SYS_GETTID => SyscallResult::Continue(1),
        SYS_GETRANDOM => {
            // Fill buffer with zeros (deterministic)
            let buf = a0;
            let len = a1 as usize;
            let host = space.g2h(buf);
            unsafe {
                std::ptr::write_bytes(host, 0, len);
            }
            SyscallResult::Continue(a1)
        }
        // Return -ENOSYS for unimplemented
        SYS_RSEQ | SYS_RISCV_HWPROBE => SyscallResult::Continue(ENOSYS),
        SYS_FUTEX => do_futex(space, a0, a1, a2),
        SYS_TGKILL => {
            // sig = a2; SIGABRT = 6
            if a2 == 6 {
                SyscallResult::Exit(128 + 6)
            } else {
                SyscallResult::Continue(0)
            }
        }
        SYS_WRITEV => do_writev(space, a0, a1, a2),
        SYS_IOCTL => SyscallResult::Continue(ENOTTY),
        SYS_FSTAT => do_fstat(space, a0, a1),
        SYS_PRLIMIT64 => do_prlimit64(space, a0, a1, a2, a3),
        SYS_UNAME => do_uname(space, a0),
        SYS_READLINKAT => do_readlinkat(space, a0, a1, a2, a3, elf_path),
        SYS_CLOCK_GETTIME => do_clock_gettime(space, a0, a1),
        _ => {
            eprintln!("[tcg] unknown syscall {nr} → -ENOSYS");
            SyscallResult::Continue(ENOSYS)
        }
    }
}

// ---------------------------------------------------------------
// Helper: convert libc errno to negative return
// ---------------------------------------------------------------

fn errno_ret() -> u64 {
    let e = unsafe { *libc::__errno_location() };
    (-e as i64) as u64
}

// ---------------------------------------------------------------
// writev(fd, iov, iovcnt)
// ---------------------------------------------------------------

fn do_writev(
    space: &mut GuestSpace,
    fd: u64,
    iov_addr: u64,
    iovcnt: u64,
) -> SyscallResult {
    let fd = fd as i32;
    let cnt = iovcnt as usize;
    let mut total: usize = 0;
    // Each guest iovec is 16 bytes: u64 base + u64 len
    for i in 0..cnt {
        let entry = iov_addr + (i as u64) * 16;
        let base = unsafe { *(space.g2h(entry) as *const u64) };
        let len = unsafe { *(space.g2h(entry + 8) as *const u64) } as usize;
        if len == 0 {
            continue;
        }
        let host = space.g2h(base);
        let ret = unsafe { libc::write(fd, host as *const libc::c_void, len) };
        if ret < 0 {
            return SyscallResult::Continue(errno_ret());
        }
        total += ret as usize;
    }
    SyscallResult::Continue(total as u64)
}

// ---------------------------------------------------------------
// fstat(fd, statbuf)
// ---------------------------------------------------------------

fn do_fstat(space: &mut GuestSpace, fd: u64, buf_addr: u64) -> SyscallResult {
    // RISC-V struct stat is 128 bytes.
    // For stdio fds, return a char device stub.
    let fd = fd as i32;
    let host_buf = space.g2h(buf_addr);
    unsafe {
        std::ptr::write_bytes(host_buf, 0, 128);
    }
    if (0..=2).contains(&fd) {
        // st_mode = S_IFCHR | 0o666 at offset 16
        let mode: u32 = 0o020666; // S_IFCHR | rw-rw-rw-
        unsafe {
            let p = host_buf.add(16) as *mut u32;
            p.write_unaligned(mode);
        }
        SyscallResult::Continue(0)
    } else {
        // Forward to host fstat
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::fstat(fd, &mut st) };
        if ret < 0 {
            return SyscallResult::Continue(errno_ret());
        }
        // Fill RISC-V stat layout (LP64):
        //  0: st_dev (u64)
        //  8: st_ino (u64)
        // 16: st_mode (u32)
        // 20: st_nlink (u32)
        // 24: st_uid (u32)
        // 28: st_gid (u32)
        // 32: st_rdev (u64)
        // 40: __pad1 (u64)
        // 48: st_size (i64)
        // 56: st_blksize (i32)
        // 60: __pad2 (i32)
        // 64: st_blocks (i64)
        // 72: st_atime (i64)
        // 80: st_atime_nsec (i64)
        // 88: st_mtime (i64)
        // 96: st_mtime_nsec (i64)
        // 104: st_ctime (i64)
        // 112: st_ctime_nsec (i64)
        unsafe {
            let p = host_buf;
            *(p as *mut u64) = st.st_dev;
            *(p.add(8) as *mut u64) = st.st_ino;
            *(p.add(16) as *mut u32) = st.st_mode;
            *(p.add(20) as *mut u32) = st.st_nlink as u32;
            *(p.add(24) as *mut u32) = st.st_uid;
            *(p.add(28) as *mut u32) = st.st_gid;
            *(p.add(32) as *mut u64) = st.st_rdev;
            *(p.add(48) as *mut i64) = st.st_size;
            *(p.add(56) as *mut i32) = st.st_blksize as i32;
            *(p.add(64) as *mut i64) = st.st_blocks;
            *(p.add(72) as *mut i64) = st.st_atime;
            *(p.add(80) as *mut i64) = st.st_atime_nsec;
            *(p.add(88) as *mut i64) = st.st_mtime;
            *(p.add(96) as *mut i64) = st.st_mtime_nsec;
            *(p.add(104) as *mut i64) = st.st_ctime;
            *(p.add(112) as *mut i64) = st.st_ctime_nsec;
        }
        SyscallResult::Continue(0)
    }
}

// ---------------------------------------------------------------
// prlimit64(pid, resource, new_rlim, old_rlim)
// ---------------------------------------------------------------

fn do_prlimit64(
    space: &mut GuestSpace,
    _pid: u64,
    resource: u64,
    _new_rlim: u64,
    old_rlim: u64,
) -> SyscallResult {
    const RLIMIT_STACK: u64 = 3;
    const RLIM_INFINITY: u64 = u64::MAX;
    if old_rlim != 0 {
        let p = space.g2h(old_rlim);
        if resource == RLIMIT_STACK {
            // rlim_cur = 8 MB, rlim_max = RLIM_INFINITY
            unsafe {
                *(p as *mut u64) = 8 * 1024 * 1024;
                *(p.add(8) as *mut u64) = RLIM_INFINITY;
            }
        } else {
            // Forward to host
            let mut rl: libc::rlimit = unsafe { std::mem::zeroed() };
            let ret = unsafe {
                libc::getrlimit(resource as libc::__rlimit_resource_t, &mut rl)
            };
            if ret < 0 {
                return SyscallResult::Continue(errno_ret());
            }
            unsafe {
                *(p as *mut u64) = rl.rlim_cur;
                *(p.add(8) as *mut u64) = rl.rlim_max;
            }
        }
    }
    SyscallResult::Continue(0)
}

// ---------------------------------------------------------------
// uname(buf)
// ---------------------------------------------------------------

fn do_uname(space: &mut GuestSpace, buf_addr: u64) -> SyscallResult {
    // new_utsname: 6 fields × 65 bytes = 390 bytes
    let p = space.g2h(buf_addr);
    unsafe {
        std::ptr::write_bytes(p, 0, 390);
    }
    let fields: [&[u8]; 6] = [
        b"Linux",   // sysname
        b"tcg-rs",  // nodename
        b"6.1.0",   // release
        b"#1 SMP",  // version
        b"riscv64", // machine
        b"(none)",  // domainname
    ];
    for (i, val) in fields.iter().enumerate() {
        let dst = unsafe { p.add(i * 65) };
        let len = val.len().min(64);
        unsafe {
            std::ptr::copy_nonoverlapping(val.as_ptr(), dst, len);
        }
    }
    SyscallResult::Continue(0)
}

// ---------------------------------------------------------------
// readlinkat(dirfd, pathname, buf, bufsiz)
// ---------------------------------------------------------------

fn do_readlinkat(
    space: &mut GuestSpace,
    _dirfd: u64,
    path_addr: u64,
    buf_addr: u64,
    bufsiz: u64,
    elf_path: &str,
) -> SyscallResult {
    // Read guest path string
    let host_path = space.g2h(path_addr);
    let path = unsafe { std::ffi::CStr::from_ptr(host_path as *const i8) };
    let path_bytes = path.to_bytes();
    if path_bytes == b"/proc/self/exe" {
        let elf = elf_path.as_bytes();
        let len = elf.len().min(bufsiz as usize);
        let dst = space.g2h(buf_addr);
        unsafe {
            std::ptr::copy_nonoverlapping(elf.as_ptr(), dst, len);
        }
        SyscallResult::Continue(len as u64)
    } else {
        SyscallResult::Continue(ENOENT)
    }
}

// ---------------------------------------------------------------
// clock_gettime(clk_id, tp)
// ---------------------------------------------------------------

fn do_clock_gettime(
    space: &mut GuestSpace,
    clk_id: u64,
    tp_addr: u64,
) -> SyscallResult {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::clock_gettime(clk_id as i32, &mut ts) };
    if ret < 0 {
        return SyscallResult::Continue(errno_ret());
    }
    // Guest timespec: i64 tv_sec + i64 tv_nsec = 16 bytes
    let p = space.g2h(tp_addr);
    unsafe {
        *(p as *mut i64) = ts.tv_sec;
        *(p.add(8) as *mut i64) = ts.tv_nsec;
    }
    SyscallResult::Continue(0)
}

// ---------------------------------------------------------------
// futex(uaddr, op, val, ...) — single-threaded stub
// ---------------------------------------------------------------

fn do_futex(
    space: &mut GuestSpace,
    uaddr: u64,
    op: u64,
    _val: u64,
) -> SyscallResult {
    const FUTEX_CMD_MASK: u64 = 0x7f;
    const FUTEX_WAIT: u64 = 0;
    const FUTEX_WAKE: u64 = 1;
    const EAGAIN: u64 = (-11i64) as u64;
    let _ = space.g2h(uaddr); // validate addr

    match op & FUTEX_CMD_MASK {
        FUTEX_WAIT => {
            // Single-threaded: no one to wake us.
            SyscallResult::Continue(EAGAIN)
        }
        FUTEX_WAKE => {
            // No waiters in single-threaded mode.
            SyscallResult::Continue(0)
        }
        _ => SyscallResult::Continue(ENOSYS),
    }
}
