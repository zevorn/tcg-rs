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
CFLAGS   = -static -nostdlib -march=rv64im -mabi=lp64 -O2
BUILDDIR = build/riscv64

HAS_CC := $(shell command -v $(CC) 2>/dev/null)

SRCS = $(wildcard riscv/*.c)
BINS = $(patsubst riscv/%.c,$(BUILDDIR)/%,$(SRCS))

ifdef HAS_CC
all: $(BINS)
else
all:
	@echo "SKIP: $(CC) not found, install with:"
	@echo "  apt install gcc-riscv64-linux-gnu"
endif

$(BUILDDIR)/%: riscv/%.c | $(BUILDDIR)
	$(CC) $(CFLAGS) -o $@ $<

$(BUILDDIR):
	mkdir -p $@

clean:
	rm -rf build

.PHONY: all clean
