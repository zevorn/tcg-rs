use tcg_linux_user::guest_space::{
    page_align_down, page_align_up, page_size, GuestSpace,
};

#[test]
fn test_create_and_drop() {
    let space = GuestSpace::new().unwrap();
    assert!(!space.guest_base().is_null());
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
    let readback =
        unsafe { std::slice::from_raw_parts(host as *const u8, data.len()) };
    assert_eq!(readback, data);
}

#[test]
fn test_page_align() {
    let ps = page_size() as u64;
    assert_eq!(page_align_up(0), 0);
    assert_eq!(page_align_up(1), ps);
    assert_eq!(page_align_up(ps), ps);
    assert_eq!(page_align_up(ps + 1), ps * 2);
    assert_eq!(page_align_down(ps - 1), 0);
    assert_eq!(page_align_down(ps), ps);
}
