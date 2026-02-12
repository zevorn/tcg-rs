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
LIBC_SRCS   = riscv/hello_printf.c

BARE_BINS = $(patsubst riscv/%.c,$(BUILDDIR)/%,$(BARE_SRCS))
LIBC_BINS = $(patsubst riscv/%.c,$(BUILDDIR)/%,$(LIBC_SRCS))

HAS_CC := $(shell command -v $(CC) 2>/dev/null)

ifdef HAS_CC
all: $(BARE_BINS) $(LIBC_BINS)
else
all:
	@echo "SKIP: $(CC) not found, install with:"
	@echo "  apt install gcc-riscv64-linux-gnu"
endif

$(BARE_BINS): $(BUILDDIR)/%: riscv/%.c | $(BUILDDIR)
	$(CC) $(BARE_CFLAGS) -o $@ $<

$(LIBC_BINS): $(BUILDDIR)/%: riscv/%.c | $(BUILDDIR)
	$(CC) $(LIBC_CFLAGS) -o $@ $<

$(BUILDDIR):
	mkdir -p $@

clean:
	rm -rf ../../target/guest

.PHONY: all clean
