Real-Time Ring Buffer
=====================

A wait-free single-producer single-consumer (SPSC) ring buffer for Rust.

* Crate: https://crates.io/crates/rtrb
* Documentation: https://docs.rs/rtrb

This crate can be used without the standard library (`#![no_std]`)
by disabling the `std` feature (which is enabled by default),
but the [alloc](https://doc.rust-lang.org/alloc/) crate is needed nevertheless.


Usage
-----

Add this to your `Cargo.toml`:

```toml
[dependencies]
rtrb = "0.2"
```


Breaking Changes
----------------

For a list of breaking changes
and for instructions how to upgrade between released versions,
have a look at the [changelog](https://github.com/mgeier/rtrb/releases).


Development
-----------

Running the tests:

    cargo test

Testing the benchmarks (without actually benchmarking):

    cargo test --benches

Running the benchmarks (using the [criterion](https://docs.rs/criterion/) crate;
results will be available in `target/criterion/report/index.html`):

    cargo bench

Creating the HTML docs (which will be available in `target/doc/rtrb/index.html`):

    cargo doc

To measure code coverage, nightly Rust is required, as well as a few additional dependencies:

    rustup toolchain install nightly
    rustup component add llvm-tools-preview
    cargo install grcov

Test coverage data can be obtained and analyzed with these commands:

    cargo clean
    RUSTFLAGS="-Z instrument-coverage" RUSTDOCFLAGS="-Z instrument-coverage -Z unstable-options --persist-doctests target/debug/doctestbins" LLVM_PROFILE_FILE="coverage/%p-%m.profraw" cargo +nightly test
    grcov coverage --source-dir . --binary-path target/debug --output-type html --output-path coverage

The last command creates an HTML report in `coverage/index.html`.

Testing with Miri also needs nightly Rust:

    cargo +nightly miri test

Running the tests with ThreadSanitizer requires nightly Rust as well:

    RUSTFLAGS="-Z sanitizer=thread" cargo +nightly test --tests -Z build-std --target x86_64-unknown-linux-gnu

You might have to adapt the `--target` option to your system (see e.g. `rustup show`).


Minimum Supported `rustc` Version
---------------------------------

This crate's minimum supported `rustc` version (MSRV) is `1.44.0`.
The MSRV is not expected to be updated frequently, but if it is,
there will be (at least) a *minor* version bump.


Origin Story
------------

The initial code has been ripped off of https://github.com/crossbeam-rs/crossbeam/pull/338,
with permission of the PR author.

It has been isolated from the rest of `crossbeam` with [git-filter-repo]:

    git-filter-repo --subdirectory-filter crossbeam-queue --path src/spsc.rs --path tests/spsc.rs --refs refs/heads/spsc

[git-filter-repo]: https://github.com/newren/git-filter-repo


Alternatives
------------

If you don't like this crate, no problem, there are several alternatives for you to choose from.
There are many varieties of ring buffers available, here we limit the selection
to wait-free SPSC implementations:

* [fixed-queue](https://crates.io/crates/fixed-queue) (using const generics, see `fixed_queue::spsc`)
* [heapless](https://crates.io/crates/heapless) (for embedded systems, see `heapless::spsc`)
* [jack](https://crates.io/crates/jack) (FFI bindings for JACK, see `jack::Ringbuffer`)
* [magnetic](https://crates.io/crates/magnetic) (see `magnetic::spsc` module)
* [npnc](https://crates.io/crates/npnc) (see `npnc::bounded::spsc` module)
* [ringbuf](https://crates.io/crates/ringbuf)
* [shmem-ipc](https://crates.io/crates/shmem-ipc) (see `shmem_ipc::sharedring` and `shmem_ipc::ringbuf` modules)

There are also implementations in other languages:

* [boost::lockfree::spsc_queue](https://www.boost.org/doc/libs/master/doc/html/boost/lockfree/spsc_queue.html) (C++)
* [folly::ProducerConsumerQueue](https://github.com/facebook/folly/blob/main/folly/docs/ProducerConsumerQueue.md) (C++)
* [JACK ring buffer](https://jackaudio.org/api/ringbuffer_8h.html)  (C)
* [PortAudio ring buffer](http://portaudio.com/docs/v19-doxydocs-dev/pa__ringbuffer_8h.html) (C)
* [readerwriterqueue](https://github.com/cameron314/readerwriterqueue) (C++)
* [ringbuf.js](https://github.com/padenot/ringbuf.js) (JavaScript, using `SharedArrayBuffer`)
* [SPSCQueue](https://github.com/rigtorp/SPSCQueue) (C++)

If you know more alternatives for this list,
please [open an issue](https://github.com/mgeier/rtrb/issues).


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
