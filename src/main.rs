extern crate semver;

use semver::Version;
use std::env;
use std::path::{PathBuf, Path};
use std::sync::mpsc::{self, Sender, Receiver};

fn main() {
    report_results(run());
}

fn run() -> Result<TestResults, Error> {
    let options = try!(parse_options());
    let config = try!(get_config(options));

    let rev_deps = try!(get_rev_deps(&config.crate_name));
    let crates = try!(acquire_crates(&config));
    let results = TestResults::new();
    for rev_dep in rev_deps {
        let result = run_test(&crates.base, &crates.next, rev_dep);
        results.add(result);
    }

    if results.failed() {
        Err(Error::TestFailure(results))
    } else {
        Ok(results)
    }
}

enum Options {
    Implicit,
    Base(Rev),
    Explicit(Rev, Rev)
}

fn parse_options() -> Result<Options, Error> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 2 {
        Ok(Options::Implicit)
    } else if args.len() < 3 {
        Ok(Options::Base(try!(parse_rev(&args[1]))))
    } else if args.len() == 3 {
        Ok(Options::Explicit(try!(parse_rev(&args[1])),
                             try!(parse_rev(&args[2]))))
    } else {
        Err(Error::BadArgs)
    }
}

fn parse_rev(s: &str) -> Result<Rev, Error> {
    match Version::parse(s) {
        Ok(v) => Ok(Rev::Published(v)),
        Err(_) => {
            if !s.contains("Cargo.toml") {
                Err(Error::ManifestName(s.to_string()))
            } else {
                Ok(Rev::Source(PathBuf::from(s)))
            }
        }
    }
}

struct Config {
    crate_name: String,
    base_rev: Rev,
    next_rev: Rev
}

enum Rev {
    Published(Version),
    Source(PathBuf)
}

type VersionNumber = String;

fn get_config(options: Options) -> Result<Config, Error> {
    unimplemented!()
}

struct RevDep;

fn get_rev_deps(crate_name: &str) -> Result<Vec<RevDep>, Error> {
    unimplemented!()
}

struct Crates {
    base: PathBuf,
    next: PathBuf
}

fn acquire_crates(config: &Config) -> Result<Crates, Error> {
    unimplemented!()
}

impl Drop for Crates {
    fn drop(&mut self) {
        unimplemented!()
    }
}

struct TestResults;

impl TestResults {
    fn new() -> TestResults {
        unimplemented!()
    }

    fn add(&self, result: TestResultFuture) {
        unimplemented!()
    }

    fn failed(&self) -> bool {
        unimplemented!()
    }
}

enum TestResult {
    Broken(CompileResult),
    Fail(CompileResult, CompileResult),
    Pass(CompileResult, CompileResult)
}

struct TestResultFuture;

fn new_result_future() -> (Sender<TestResult>, TestResultFuture) {
    unimplemented!()
}

fn run_test(base_crate: &Path, next_crate: &Path, rev_dep: RevDep) -> TestResultFuture {
    let (result_tx, result_future) = new_result_future();
    let base_crate = base_crate.clone();
    let next_crate = next_crate.clone();
    spawn(move || {
        let res = run_test_local(base_crate, next_crate, rev_dep);
        result_tx.send(res);
    });

    return result_future;
}

fn spawn<F: FnOnce() + Send>(f: F) {
}

fn run_test_local(base_crate: &Path, next_crate: &Path, ref rev_dep: RevDep) -> TestResult {
    let base_result = compile_with_custom_dep(rev_dep, base_crate);

    if base_result.failed() {
        return TestResult::Broken(base_result);
    }
    let next_result = compile_with_custom_dep(rev_dep, next_crate);

    if next_result.failed() {
        TestResult::Fail(base_result, next_result)
    } else {
        TestResult::Pass(base_result, next_result)
    }
}

struct CompileResult {
    stdout: String,
    stderr: String,
    success: bool
}

impl CompileResult {
    fn failed(&self) -> bool { unimplemented!() }
}

fn compile_with_custom_dep(rev_dep: &RevDep, krate: &Path) -> CompileResult {
    unimplemented!()
}

fn report_results(res: Result<TestResults, Error>) {
}

enum Error {
    BadArgs,
    ManifestName(String),
    TestFailure(TestResults),
}
