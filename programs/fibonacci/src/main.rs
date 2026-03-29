//! A simple SP1 guest that commits the input `n`, `fib(n)`, and `fib(n + 1)`.

#![no_main]
sp1_zkvm::entrypoint!(main);

pub fn main() {
    let n = sp1_zkvm::io::read::<u32>();

    sp1_zkvm::io::commit(&n);

    let mut a = 0u32;
    let mut b = 1u32;
    for _ in 0..n {
        let mut c = a + b;
        c %= 7919;
        a = b;
        b = c;
    }

    sp1_zkvm::io::commit(&a);
    sp1_zkvm::io::commit(&b);
}
