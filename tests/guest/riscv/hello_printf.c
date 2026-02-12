// RISC-V hello world using standard C library.
// Requires static glibc (rv64gc, lp64d ABI).

#include <stdio.h>

int main(void) {
    printf("Hello, World!\n");
    return 0;
}
