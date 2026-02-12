# Build guest test programs for tcg-rs.
#
# Usage:
#   make                  # build all targets
#   make clean            # remove build artifacts
#
# Requires riscv64-linux-gnu-gcc (apt install gcc-riscv64-linux-gnu).
# Skips gracefully when the cross-compiler is not found.

CROSS_COMPILE ?= riscv64-linux-gnu-
CC       = $(CROSS_COMPILE)gcc
BUILDDIR = ../../target/guest/riscv64

# Bare-metal programs (no libc, custom _start).
BARE_CFLAGS = -static -nostdlib -march=rv64im -mabi=lp64 -O2
BARE_SRCS   = riscv/hello.c

# Programs linked with static glibc.
LIBC_CFLAGS = -static -march=rv64gc -mabi=lp64d -O2
LIBC_SRCS   = riscv/hello_printf.c riscv/hello_float.c riscv/argv_echo.c
LIBC_MULTI_BINS = $(BUILDDIR)/dhrystone

BARE_BINS = $(patsubst riscv/%.c,$(BUILDDIR)/%,$(BARE_SRCS))
LIBC_BINS = $(patsubst riscv/%.c,$(BUILDDIR)/%,$(LIBC_SRCS))

HAS_CC := $(shell command -v $(CC) 2>/dev/null)

ifdef HAS_CC
all: $(BARE_BINS) $(LIBC_BINS) $(LIBC_MULTI_BINS)
else
all:
	@echo "SKIP: $(CC) not found, install with:"
	@echo "  apt install gcc-riscv64-linux-gnu"
endif

$(BARE_BINS): $(BUILDDIR)/%: riscv/%.c | $(BUILDDIR)
	$(CC) $(BARE_CFLAGS) -o $@ $<

$(LIBC_BINS): $(BUILDDIR)/%: riscv/%.c | $(BUILDDIR)
	$(CC) $(LIBC_CFLAGS) -o $@ $<

DHRY_DIR = riscv/dhrystone

# Dhrystone 2.1 sources are vendored from
# https://github.com/sifive/benchmark-dhrystone.
$(BUILDDIR)/dhrystone: $(DHRY_DIR)/dhry_1.c $(DHRY_DIR)/dhry_2.c $(DHRY_DIR)/dhry.h | $(BUILDDIR)
	$(CC) $(LIBC_CFLAGS) -std=gnu89 -DTIME -o $@ $(DHRY_DIR)/dhry_1.c $(DHRY_DIR)/dhry_2.c

$(BUILDDIR):
	mkdir -p $@

clean:
	rm -rf ../../target/guest

.PHONY: all clean
