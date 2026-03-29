# Fibonacci SP1 Program

This directory contains the guest source for the Phase 3.1 SP1 smoke-test program.

## Build locally

1. Install the SP1 CLI:
   - `cargo install sp1-cli --locked`
2. Build the guest ELF from this directory:
   - `cargo prove build`
3. Copy the generated ELF into `programs/fibonacci/elf/fibonacci-program`.

The repository already vendors a prebuilt ELF at `programs/fibonacci/elf/fibonacci-program` so normal `cargo test` and CI runs do not require SP1 build tooling.
