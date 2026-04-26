Performance Comparison between `rtrb` and Other Crates
======================================================

Run all benchmarks (takes a long time):

    cargo bench

Run a single benchmark file (see `benches/` subdirectory):

    cargo bench --bench push_pop

Run all benchmarks that match a substring:

    cargo bench --bench push_pop -- eager-push

List all available benchmarks:

    cargo bench -- --list

Some results are available in
[issue #39](https://github.com/mgeier/rtrb/issues/39),
feel free to add your own results there!
