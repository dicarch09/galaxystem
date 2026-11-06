// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `gx` command-line tool (F0-diseno.md §8).
//!
//! Empty placeholder created in F0.0; implementation arrives in a later subphase.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

fn main() {
    println!("gx {}", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {
        assert_eq!(env!("CARGO_PKG_NAME"), "gx-cli");
    }
}
