// RISC-V float test using standard C library.
// Requires static glibc (rv64gc, lp64d ABI).

#include <stdio.h>

int main(void) {
    double a = 1.5;
    double b = 2.25;
    double c = a * b + 0.5;
    double d = c / 3.0;
    float f = (float)c;
    long i = (long)c;
    unsigned long u = (unsigned long)(c + 1.0);

    printf("a=%.2f b=%.2f c=%.6f d=%.6f f=%.3f i=%ld u=%lu\n",
           a, b, c, d, f, i, u);
    return 0;
}
