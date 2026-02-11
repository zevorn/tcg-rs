// Minimal RISC-V hello world using raw syscalls.
// No glibc, no compressed instructions.

static const char msg[] = "Hello, World!\n";

static long syscall3(long n, long a0, long a1, long a2) {
    register long a7 __asm__("a7") = n;
    register long _a0 __asm__("a0") = a0;
    register long _a1 __asm__("a1") = a1;
    register long _a2 __asm__("a2") = a2;
    __asm__ volatile(
        "ecall"
        : "+r"(_a0)
        : "r"(_a1), "r"(_a2), "r"(a7)
        : "memory"
    );
    return _a0;
}

static void syscall1(long n, long a0)
    __attribute__((noreturn));

static void syscall1(long n, long a0) {
    register long a7 __asm__("a7") = n;
    register long _a0 __asm__("a0") = a0;
    __asm__ volatile(
        "ecall"
        :
        : "r"(_a0), "r"(a7)
    );
    __builtin_unreachable();
}

void _start(void) {
    // write(1, msg, 14)
    syscall3(64, 1, (long)msg, sizeof(msg) - 1);
    // exit(0)
    syscall1(93, 0);
}
