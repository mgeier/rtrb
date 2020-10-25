Real-Time Ring Buffer
=====================

Ripped off of https://github.com/crossbeam-rs/crossbeam/pull/338.

Isolated from the rest of `crossbeam` with [git-filter-repo]:

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
