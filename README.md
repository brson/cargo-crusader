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

When you run `cargo-crusader' from the source directory of your
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
