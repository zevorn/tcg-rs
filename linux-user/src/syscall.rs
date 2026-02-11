use crate::guest_space::GuestSpace;

// RISC-V Linux syscall numbers
const SYS_WRITE: u64 = 64;
const SYS_EXIT: u64 = 93;
const SYS_EXIT_GROUP: u64 = 94;
const SYS_BRK: u64 = 214;
const SYS_MMAP: u64 = 222;
const SYS_MPROTECT: u64 = 226;
const SYS_MUNMAP: u64 = 215;
const SYS_SET_TID_ADDRESS: u64 = 96;
const SYS_SET_ROBUST_LIST: u64 = 99;
const SYS_PRLIMIT64: u64 = 261;
const SYS_GETRANDOM: u64 = 278;
const SYS_RSEQ: u64 = 293;
const SYS_RT_SIGACTION: u64 = 134;
const SYS_RT_SIGPROCMASK: u64 = 135;
const SYS_GETPID: u64 = 172;
const SYS_GETTID: u64 = 178;
const SYS_UNAME: u64 = 160;
const SYS_READLINKAT: u64 = 78;
const SYS_IOCTL: u64 = 29;
const SYS_WRITEV: u64 = 66;
const SYS_CLOSE: u64 = 57;
const SYS_FSTAT: u64 = 80;
const SYS_CLOCK_GETTIME: u64 = 113;
const SYS_MADVISE: u64 = 233;

const ENOSYS: u64 = (-38i64) as u64;

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
) -> SyscallResult {
    let nr = regs[17]; // a7
    let a0 = regs[10];
    let a1 = regs[11];
    let a2 = regs[12];

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
        SYS_RSEQ | SYS_PRLIMIT64 | SYS_UNAME | SYS_READLINKAT | SYS_IOCTL
        | SYS_WRITEV | SYS_FSTAT | SYS_CLOCK_GETTIME => {
            eprintln!(
                "[tcg] unimplemented syscall {nr} \
                 → -ENOSYS"
            );
            SyscallResult::Continue(ENOSYS)
        }
        _ => {
            eprintln!("[tcg] unknown syscall {nr} → -ENOSYS");
            SyscallResult::Continue(ENOSYS)
        }
    }
}
