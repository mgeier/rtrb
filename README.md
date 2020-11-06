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


Origin Story
------------

The initial code has been ripped off of https://github.com/crossbeam-rs/crossbeam/pull/338,
with permission of the PR author [@stjepang](https://github.com/stjepang).

It has been isolated from the rest of `crossbeam` with [git-filter-repo]:

    git-filter-repo --subdirectory-filter crossbeam-queue --path src/spsc.rs --path tests/spsc.rs --refs refs/heads/spsc

[git-filter-repo]: https://github.com/newren/git-filter-repo


Alternatives
------------

If you don't like this crate, no problem, there are several alternatives for you to choose from.
There are many varieties of ring buffers available, here we limit the selection
to wait-free SPSC implementations:

* [fdringbuf](https://crates.io/crates/fdringbuf) (see `fdringbuf::ringbuf` module)
* [jack](https://crates.io/crates/jack) (FFI bindings for JACK, see `jack::Ringbuffer`)
* [npnc](https://crates.io/crates/npnc) (see `npnc::bounded::spsc` module)
* [ringbuf](https://crates.io/crates/ringbuf)
* [spsc-bounded-queue](https://crates.io/crates/spsc-bounded-queue)

There are also implementations in other languages:

* [boost::lockfree::spsc_queue](https://www.boost.org/doc/libs/master/doc/html/boost/lockfree/spsc_queue.html) (C++)
* [JACK ring buffer](https://jackaudio.org/api/ringbuffer_8h.html)  (C)
* [PortAudio ring buffer](http://portaudio.com/docs/v19-doxydocs-dev/pa__ringbuffer_8h.html) (C)
* [readerwriterqueue](https://github.com/cameron314/readerwriterqueue) (C++)
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
