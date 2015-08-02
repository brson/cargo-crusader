extern crate env_logger;
#[macro_use]
extern crate log;
extern crate semver;
extern crate toml;

use semver::Version;
use std::convert::From;
use std::env;
use std::fs::File;
use std::io::{self, Read};
use std::path::{PathBuf, Path};
use std::sync::mpsc::{self, Sender, Receiver};

fn main() {
    env_logger::init().unwrap();
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
    Base(Version),
    Explicit(Version, Version)
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

fn parse_rev(s: &str) -> Result<Version, Error> {
    Ok(try!(Version::parse(s)))
}

struct Config {
    manifest_path: PathBuf,
    crate_name: String,
    base_origin: Origin,
    next_origin: Origin
}

enum Origin {
    Published(Version),
    Source(PathBuf)
}

type VersionNumber = String;

fn get_config(options: Options) -> Result<Config, Error> {
    let manifest = env::var("CRUSADER_MANIFEST");
    let manifest = manifest.unwrap_or_else(|_| "./Cargo.toml".to_string());
    let manifest = PathBuf::from(manifest);

    let source_name = try!(get_crate_name(&manifest));
    match options {
        Options::Implicit => {
            let rev = try!(get_most_recent_rev(&source_name));
            Ok(Config {
                manifest_path: manifest.clone(),
                crate_name: source_name,
                base_origin: Origin::Published(rev),
                next_origin: Origin::Source(manifest)
            })
        }
        Options::Base(base_rev) => {
            Ok(Config {
                manifest_path: manifest.clone(),
                crate_name: source_name,
                base_origin: Origin::Published(base_rev),
                next_origin: Origin::Source(manifest)
            })
        }
        Options::Explicit(base_rev, next_rev) => {
            Ok(Config {
                manifest_path: manifest.clone(),
                crate_name: source_name,
                base_origin: Origin::Published(base_rev),
                next_origin: Origin::Published(next_rev)
            })
        }
    }
}

fn get_crate_name(manifest_path: &Path) -> Result<String, Error> {
    let ref toml = try!(load_string(manifest_path));
    let mut parser = toml::Parser::new(toml);
    let toml = parser.parse();
    let map = if toml.is_none() {
        return Err(Error::TomlError(parser.errors))
    } else {
        toml.unwrap()
    };

    match map.get("package") {
        Some(&toml::Value::Table(ref t)) => {
            match t.get("name") {
                Some(&toml::Value::String(ref s)) => {
                    Ok(s.clone())
                }
                _ => {
                    Err(Error::ManifestName(PathBuf::from(manifest_path)))
                }
            }
        }
        _ => {
            Err(Error::ManifestName(PathBuf::from(manifest_path)))
        }
    }
}

fn load_string(path: &Path) -> Result<String, Error> {
    let mut file = try!(File::open(path));
    let mut s = String::new();
    try!(file.read_to_string(&mut s));
    Ok(s)
}

fn get_most_recent_rev(crate_name: &str) -> Result<Version, Error> {
    // TODO
    Ok(try!(Version::parse("0.0.0")))
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

#[derive(Debug)]
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

#[derive(Debug)]
enum TestResult {
    Broken(CompileResult),
    Fail(CompileResult, CompileResult),
    Pass(CompileResult, CompileResult)
}

#[derive(Debug)]
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

#[derive(Debug)]
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
    println!("results: {:?}", res);
}

#[derive(Debug)]
enum Error {
    BadArgs,
    ManifestName(PathBuf),
    TestFailure(TestResults),
    SemverError(semver::ParseError),
    TomlError(Vec<toml::ParserError>),
    IoError(io::Error),
}

impl From<semver::ParseError> for Error {
    fn from(e: semver::ParseError) -> Error {
        Error::SemverError(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::IoError(e)
    }
}
