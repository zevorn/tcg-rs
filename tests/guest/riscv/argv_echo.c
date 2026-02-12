// Print guest argc/argv to verify startup argument passing.

#include <stdio.h>

int main(int argc, char **argv) {
    printf("argc=%d\n", argc);
    for (int i = 1; i < argc; ++i) {
        printf("arg%d=%s\n", i, argv[i]);
    }
    return 0;
}
