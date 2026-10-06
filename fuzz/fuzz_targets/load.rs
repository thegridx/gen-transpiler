//! The loader rejects a garbled artifact with an error, never a panic. Loading copies and maps
//! the artifact but runs none of it.
#![no_main]

use std::collections::HashMap;

use libfuzzer_sys::fuzz_target;
use transpiler::execute::{LoaderContext, TranspiledVm};

fuzz_target!(|data: &[u8]| {
    let _result = TranspiledVm::load(data, LoaderContext::Trusted, &HashMap::new());
});
