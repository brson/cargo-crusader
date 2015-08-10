# Cargo Crusader

Hark, Rust crate author! The battle for Rust's reputation as *The Most
Reliable Software Platform Ever* is here, and nobody is free of
responsibility. The future of Rust, dear Rustilian, is in your hands.

Join the **Cargo Crusade** and bring the [Theory of Responsible API
Evolution][evo] to the non-believers.

Cargo Crusader is a tool to help crate authors evaluate the impact of
future API changes on downstream users of that crate before they are
published to [crates.io].

[evo]: https://github.com/rust-lang/rfcs/blob/master/text/1105-api-evolution.md
[crates.io]: http://crates.io
[semver]: http://semver.org/

# How?

When you run `cargo-crusader` from the source directory of your
published crate, Crusader asks crates.io for all of its reverse
dependencies - *published crates that DEPEND ON YOU*. It then
downloads each of them, and builds them: first against your crate as
currently published, then against your local work-in-progress
(i.e. the next version you are going to publish). It then reports
differences in behavior.

# Getting Started

**IMPORTANT SECURITY WARNING: This program executes arbitrary
  untrusted code downloaded from the Internet. You are strongly
  recommended to take your own sandboxing precautions before running
  it.**

First, download and build Cargo Crusader, and put the `cargo-crusader`
command in your `PATH` environment variable:

```sh
$ git clone https://github.com/brson/cargo-crusader
$ cd cargo-crusader
$ cargo build --release
$ export PATH=$PATH:(`pwd`)/target/release/
```

Now change directories to your source and run `cargo-crusader`:

```sh
$ cargo-crusader
crusader: downloading reverse deps for hyper
crusader: 10 reverse deps
crusader: testing crate aloft
crusader: testing crate austenite
crusader: result 1 of 10, aloft 0.3.1: broken
crusader: testing crate bare
crusader: result 2 of 10, austenite 0.0.1: broken
crusader: testing crate catapult
crusader: result 3 of 10, bare 0.0.1: broken
crusader: testing crate chan
crusader: result 4 of 10, catapult 0.1.2: broken
crusader: testing crate chatbot
crusader: result 5 of 10, chan 0.1.14: passed
crusader: testing crate click_and_load
crusader: result 6 of 10, chatbot 0.2.2: regressed
crusader: testing crate coinbaser
crusader: result 7 of 10, click_and_load 0.0.1: broken
crusader: testing crate doapi
crusader: result 8 of 10, coinbaser 0.1.0: regressed
crusader: testing crate ease
crusader: result 9 of 10, doapi 0.1.0: broken
crusader: result 10 of 10, ease 0.2.1: regressed

passed: 1
regressed: 3
broken: 6
error: 0

full report: ./crusader-report.html
```

A full run will take quite a while. After its done it will print a
summary, as well as produce an HTML file containing the full results,
including all the compiler output for each test.

Tests result in four possible statuses: 'passed', if the reverse
dependency built both before and after the upgrade; 'regressed', if it
built before but not after; 'broken', if it didn't even build before
upgrading; and 'error', for internal Crusader errors.

# Future improvements

Presently Crusader will override reverse dependencies with your local
revision *even if the version they requested is not semver compatible
with your work-in-progress*. Crusader might first verify whether or
not the WIP qualifies as a semver-valid upgrade.

Testing upstream as well - Crusader could ask for all the WIP branches
of your dependencies and then override your build to see if upcoming
changes are going to break your crate.

Sandboxing.

# License

MIT/Apache-2.0 is the official license of both The Rust Project and The Cargo Crusade.
