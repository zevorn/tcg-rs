use std::io;
use std::ptr;

/// Guest address space size: 1 GiB.
const GUEST_SPACE_SIZE: usize = 1 << 30;

/// Default guest stack top address.
pub const GUEST_STACK_TOP: u64 = 0x3FFF_0000;

/// Default guest stack size: 8 MiB.
pub const GUEST_STACK_SIZE: usize = 8 * 1024 * 1024;

/// mmap-based guest address space.
///
/// Reserves a contiguous region of host memory and maps
/// guest addresses as offsets within it.
pub struct GuestSpace {
    base: *mut u8,
    size: usize,
    brk: u64,
}

// SAFETY: GuestSpace owns its mmap'd memory exclusively.
unsafe impl Send for GuestSpace {}

impl GuestSpace {
    /// Reserve a 1 GiB guest address space.
    pub fn new() -> io::Result<Self> {
        // SAFETY: PROT_NONE reservation, no file backing.
        let ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                GUEST_SPACE_SIZE,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            base: ptr as *mut u8,
            size: GUEST_SPACE_SIZE,
            brk: 0,
        })
    }

    /// Translate guest address to host pointer.
    #[inline]
    pub fn g2h(&self, guest_addr: u64) -> *mut u8 {
        assert!(
            (guest_addr as usize) < self.size,
            "guest addr {guest_addr:#x} out of range"
        );
        unsafe { self.base.add(guest_addr as usize) }
    }

    /// Translate host pointer to guest address.
    #[inline]
    pub fn h2g(&self, host_ptr: *const u8) -> u64 {
        let off = host_ptr as usize - self.base as usize;
        assert!(off < self.size, "host pointer not in guest space");
        off as u64
    }

    /// Base pointer for guest instruction fetch.
    #[inline]
    pub fn guest_base(&self) -> *const u8 {
        self.base as *const u8
    }

    /// Current program break (guest address).
    #[inline]
    pub fn brk(&self) -> u64 {
        self.brk
    }

    /// Set program break.
    #[inline]
    pub fn set_brk(&mut self, brk: u64) {
        self.brk = brk;
    }

    /// Map a fixed region within the guest space.
    pub fn mmap_fixed(
        &self,
        guest_addr: u64,
        size: usize,
        prot: i32,
    ) -> io::Result<()> {
        let host = self.g2h(guest_addr);
        // SAFETY: within our reserved region.
        let ret = unsafe {
            libc::mmap(
                host as *mut libc::c_void,
                size,
                prot,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED,
                -1,
                0,
            )
        };
        if ret == libc::MAP_FAILED {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Change protection on a guest region.
    pub fn mprotect(
        &self,
        guest_addr: u64,
        size: usize,
        prot: i32,
    ) -> io::Result<()> {
        let host = self.g2h(guest_addr);
        let ret =
            unsafe { libc::mprotect(host as *mut libc::c_void, size, prot) };
        if ret != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Write bytes at a guest address.
    ///
    /// # Safety
    /// The guest region must be mapped writable.
    pub unsafe fn write_bytes(&self, guest_addr: u64, data: &[u8]) {
        let dst = self.g2h(guest_addr);
        ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
    }

    /// Write a u64 at a guest address (LE).
    ///
    /// # Safety
    /// The guest region must be mapped writable.
    pub unsafe fn write_u64(&self, guest_addr: u64, val: u64) {
        let dst = self.g2h(guest_addr);
        (dst as *mut u64).write_unaligned(val);
    }

    /// Read a u64 from a guest address (LE).
    ///
    /// # Safety
    /// The guest region must be mapped readable.
    pub unsafe fn read_u64(&self, guest_addr: u64) -> u64 {
        let src = self.g2h(guest_addr);
        (src as *const u64).read_unaligned()
    }
}

impl Drop for GuestSpace {
    fn drop(&mut self) {
        if !self.base.is_null() {
            unsafe {
                libc::munmap(self.base as *mut libc::c_void, self.size);
            }
        }
    }
}

pub fn page_size() -> usize {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size <= 0 {
        4096
    } else {
        size as usize
    }
}

pub fn page_align_up(addr: u64) -> u64 {
    let ps = page_size() as u64;
    (addr + ps - 1) & !(ps - 1)
}

pub fn page_align_down(addr: u64) -> u64 {
    let ps = page_size() as u64;
    addr & !(ps - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_drop() {
        let space = GuestSpace::new().unwrap();
        assert!(!space.base.is_null());
        drop(space);
    }

    #[test]
    fn test_g2h_h2g_roundtrip() {
        let space = GuestSpace::new().unwrap();
        let addr: u64 = 0x1000;
        let host = space.g2h(addr);
        assert_eq!(space.h2g(host), addr);
    }

    #[test]
    fn test_mmap_fixed_and_write() {
        let space = GuestSpace::new().unwrap();
        let addr: u64 = 0x10000;
        let size = page_size();
        space
            .mmap_fixed(addr, size, libc::PROT_READ | libc::PROT_WRITE)
            .unwrap();

        let data = b"hello guest";
        unsafe {
            space.write_bytes(addr, data);
        }

        let host = space.g2h(addr);
        let readback = unsafe {
            std::slice::from_raw_parts(host as *const u8, data.len())
        };
        assert_eq!(readback, data);
    }

    #[test]
    fn test_page_align() {
        assert_eq!(page_align_up(0), 0);
        assert_eq!(page_align_up(1), 4096);
        assert_eq!(page_align_up(4096), 4096);
        assert_eq!(page_align_up(4097), 8192);
        assert_eq!(page_align_down(4095), 0);
        assert_eq!(page_align_down(4096), 4096);
    }
}
