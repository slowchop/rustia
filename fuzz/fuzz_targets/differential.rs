#![no_main]
use libfuzzer_sys::{fuzz_target, Corpus};
fuzz_target!(|data: &[u8]| -> Corpus {
    if rustia_fuzz::differential::fuzz_differential(data) {
        Corpus::Keep
    } else {
        Corpus::Reject
    }
});
