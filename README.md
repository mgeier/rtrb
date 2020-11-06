Real-Time Ring Buffer
=====================

Package on https://crates.io/ and documentation on https://docs.rs/ coming soon!


Development
-----------

Running the tests:

    cargo test

Testing the benchmarks (without actually benchmarking):

    cargo test --benches

Running the benchmarks (using the [criterion](https://docs.rs/criterion/) crate):

    cargo bench

Creating the HTML docs (using `nightly` Rust to enable intra-doc links):

    cargo +nightly doc


Minimum Supported `rustc` Version
---------------------------------

This crate's minimum supported `rustc` version (MSRV) is `1.36.0`.
The MSRV is not expected to be updated frequently, but if it is,
there will be (at least) a *minor* version bump.


Origin Story
------------

The initial code has been ripped off of https://github.com/crossbeam-rs/crossbeam/pull/338,
with permission of the PR author [@stjepang](https://github.com/stjepang).

It has been isolated from the rest of `crossbeam` with [git-filter-repo]:

    git-filter-repo --subdirectory-filter crossbeam-queue --path src/spsc.rs --path tests/spsc.rs --refs refs/heads/spsc

[git-filter-repo]: https://github.com/newren/git-filter-repo


License
-------

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

#### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
