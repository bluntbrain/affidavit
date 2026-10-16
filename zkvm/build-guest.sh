#!/usr/bin/env bash
# Builds the measurement guest without rzup: upstream nightly Rust already knows the
# `riscv32im-risc0-zkvm-elf` target, so std is built from source with the flags
# risc0-build 3.0 uses. The result is linked with RISC Zero's v1compat kernel by the host.
set -euo pipefail
cd "$(dirname "$0")/guest"
export CARGO_ENCODED_RUSTFLAGS=$(printf '%s\x1f' -C passes=lower-atomic -C link-arg=-Ttext=0x00200800 -C link-arg=--fatal-warnings -C panic=abort --cfg 'getrandom_backend="custom"' | sed 's/\x1f$//')
export RISC0_FEATURE_bigint2=1
export CC_riscv32im_risc0_zkvm_elf=clang
export AR_riscv32im_risc0_zkvm_elf=llvm-ar
export CFLAGS_riscv32im_risc0_zkvm_elf="--target=riscv32-unknown-elf -march=rv32im -mabi=ilp32 -nostdlib"
cargo +nightly build --release --target riscv32im-risc0-zkvm-elf \
  -Z build-std=alloc,core,proc_macro,panic_abort,std -Z build-std-features=compiler-builtins-mem "$@"
ls -la "${CARGO_TARGET_DIR:-target}/riscv32im-risc0-zkvm-elf/release/affidavit-guest"
