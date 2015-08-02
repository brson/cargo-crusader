extern crate curl;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate rustc_serialize;
extern crate semver;
extern crate toml;

use curl::{http, ErrCode};
use curl::http::Response as CurlHttpResponse;
use rustc_serialize::json;
use semver::Version;
use std::convert::From;
use std::env;
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{PathBuf, Path};
use std::str::{self, Utf8Error};
use std::sync::mpsc::{self, Sender, Receiver};

fn main() {
    env_logger::init().unwrap();
    report_results(run());
}

fn run() -> Result<TestResults, Error> {
    let config = try!(get_config());

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

struct Config {
    manifest_path: PathBuf,
    crate_name: String,
    base_origin: Origin,
    next_origin: Origin
}

enum Origin {
    Published,
    Source(PathBuf)
}

type VersionNumber = String;

fn get_config() -> Result<Config, Error> {
    let manifest = env::var("CRUSADER_MANIFEST");
    let manifest = manifest.unwrap_or_else(|_| "./Cargo.toml".to_string());
    let manifest = PathBuf::from(manifest);
    info!("Using manifest {:?}", manifest);

    let source_name = try!(get_crate_name(&manifest));
    Ok(Config {
        manifest_path: manifest.clone(),
        crate_name: source_name,
        base_origin: Origin::Published,
        next_origin: Origin::Source(manifest)
    })
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

type RevDep = String;

fn get_rev_deps(crate_name: &str) -> Result<Vec<RevDep>, Error> {
    info!("Getting reverse deps for {}", crate_name);
    let url = format!("https://crates.io/api/v1/crates/{}/reverse_dependencies", crate_name);
    let resp = try!(http::handle().get(url).exec());

    if resp.get_code() != 200 {
        return Err(Error::HttpError(CurlHttpResponseWrapper(resp)));
    }

    let body = try!(str::from_utf8(resp.get_body()));
    let rev_deps = try!(parse_rev_deps(body));

    Ok(rev_deps)
}

fn parse_rev_deps(s: &str) -> Result<Vec<RevDep>, Error> {
    #[derive(RustcEncodable, RustcDecodable)]
    struct Response {
        dependencies: Vec<Dep>,
    }

    #[derive(RustcEncodable, RustcDecodable)]
    struct Dep {
        crate_id: String
    }

    let decoded: Response = try!(json::decode(&s));

    fn depconv(d: Dep) -> RevDep { d.crate_id }

    let revdeps = decoded.dependencies.into_iter()
        .map(depconv).collect();

    info!("revdeps: {:?}", revdeps);

    Ok(revdeps)
}

struct Crates {
    base: CrateOverride,
    next: CrateOverride
}

enum CrateOverride {
    Default,
    Source(PathBuf)
}

fn acquire_crates(config: &Config) -> Result<Crates, Error> {
    let base = acquire_crate(&config.base_origin);
    let next = acquire_crate(&config.next_origin);
    Ok(Crates { base: base, next: next })
}

fn acquire_crate(origin: &Origin) -> CrateOverride {
    match *origin {
        Origin::Published => CrateOverride::Default,
        Origin::Source(ref p) => CrateOverride::Source(p.clone())
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

fn run_test(base_crate: &CrateOverride, next_crate: &CrateOverride, rev_dep: RevDep) -> TestResultFuture {
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

fn run_test_local(base_crate: &CrateOverride, next_crate: &CrateOverride, ref rev_dep: RevDep) -> TestResult {
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

fn compile_with_custom_dep(rev_dep: &RevDep, krate: &CrateOverride) -> CompileResult {
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
    CurlError(curl::ErrCode),
    HttpError(CurlHttpResponseWrapper),
    Utf8Error(Utf8Error),
    JsonDecode(json::DecoderError),
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

impl From<curl::ErrCode> for Error {
    fn from(e: curl::ErrCode) -> Error {
        Error::CurlError(e)
    }
}

impl From<Utf8Error> for Error {
    fn from(e: Utf8Error) -> Error {
        Error::Utf8Error(e)
    }
}

impl From<json::DecoderError> for Error {
    fn from(e: json::DecoderError) -> Error {
        Error::JsonDecode(e)
    }
}

struct CurlHttpResponseWrapper(CurlHttpResponse);

impl fmt::Debug for CurlHttpResponseWrapper {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let CurlHttpResponseWrapper(ref resp) = *self;
        let tup = (resp.get_code(), resp.get_headers(), resp.get_body());
        try!(fmt.write_str(&format!("{:?}", tup)));

        Ok(())
    }
}
