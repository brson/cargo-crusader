extern crate curl;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate rustc_serialize;
extern crate semver;
extern crate toml;
extern crate threadpool;
extern crate num_cpus;
extern crate tempdir;

use curl::{http, ErrCode};
use curl::http::Response as CurlHttpResponse;
use rustc_serialize::json;
use semver::Version;
use std::convert::From;
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{PathBuf, Path};
use std::process::{self, Command};
use std::str::{self, Utf8Error};
use std::string::FromUtf8Error;
use std::sync::mpsc::{self, Sender, Receiver, RecvError};
use threadpool::ThreadPool;
use tempdir::TempDir;

fn main() {
    env_logger::init().unwrap();
    report_results(run());
}

fn run() -> Result<Vec<TestResult>, Error> {
    let config = try!(get_config());

    let rev_deps = try!(get_rev_deps(&config.crate_name));
    let crates = try!(acquire_crates(&config));
    let mut results = Vec::new();
    let ref mut pool = ThreadPool::new(num_cpus::get());
    for rev_dep in rev_deps {
        let result = run_test(pool, crates.base.clone(), crates.next.clone(), rev_dep);
        results.push(result);
    }

    let results = results.into_iter().map(|r| r.take());
    let results = results.collect::<Vec<_>>();
    let failed = results.iter().any(|r| r.failed());

    if failed {
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
    debug!("Using manifest {:?}", manifest);

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

type RevDepName = String;

fn crate_url(krate: &str, call: Option<&str>) -> String {
    let url = format!("https://crates.io/api/v1/crates/{}", krate);
    match call {
        Some(c) => format!("{}/{}", url, c),
        None => url
    }
}

fn get_rev_deps(crate_name: &str) -> Result<Vec<RevDepName>, Error> {
    debug!("Getting reverse deps for {}", crate_name);
    let ref url = crate_url(crate_name, Some("reverse_dependencies"));
    let ref body = try!(http_get_to_string(url));
    let rev_deps = try!(parse_rev_deps(body));

    Ok(rev_deps)
}

fn http_get_to_string(url: &str) -> Result<String, Error> {
    Ok(try!(String::from_utf8(try!(http_get_bytes(url)))))
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, Error> {
    let resp = try!(http::handle().get(url).exec());

    if resp.get_code() == 302 {
        debug!("following 302 HTTP response");
        // Resource moved
        if let Some(l) = resp.get_headers().get("location") {
            if l.len() > 0 {
                let url = l[0].clone();
                let resp = try!(http::handle().get(url).exec());
                if resp.get_code() != 200 {
                    return Err(Error::HttpError(CurlHttpResponseWrapper(resp)));
                } else {
                    return Ok(resp.move_body());
                }
            }
        }

        return Err(Error::HttpError(CurlHttpResponseWrapper(resp)));
    } else if resp.get_code() != 200 {
        return Err(Error::HttpError(CurlHttpResponseWrapper(resp)));
    }

    Ok(resp.move_body())
}

fn parse_rev_deps(s: &str) -> Result<Vec<RevDepName>, Error> {
    #[derive(RustcEncodable, RustcDecodable)]
    struct Response {
        dependencies: Vec<Dep>,
    }

    #[derive(RustcEncodable, RustcDecodable)]
    struct Dep {
        crate_id: String
    }

    let decoded: Response = try!(json::decode(&s));

    fn depconv(d: Dep) -> RevDepName { d.crate_id }

    let revdeps = decoded.dependencies.into_iter()
        .map(depconv).collect();

    debug!("revdeps: {:?}", revdeps);

    Ok(revdeps)
}

struct Crates {
    base: CrateOverride,
    next: CrateOverride
}

#[derive(Clone)]
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

#[derive(Debug, Clone)]
struct RevDep {
    name: RevDepName,
    vers: Version
}

#[derive(Debug)]
struct TestResult {
    rev_dep: RevDep,
    data: TestResultData
}

#[derive(Debug)]
enum TestResultData {
    Broken(CompileResult),
    Fail(CompileResult, CompileResult),
    Pass(CompileResult, CompileResult),
    Error(Error),
}

impl TestResult {
    fn broken(rev_dep: RevDep, r: CompileResult) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Broken(r)
        }
    }

    fn fail(rev_dep: RevDep, r1: CompileResult, r2: CompileResult) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Fail(r1, r2)
        }
    }

    fn pass(rev_dep: RevDep, r1: CompileResult, r2: CompileResult) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Pass(r1, r2)
        }
    }

    fn error(rev_dep: RevDep, e: Error) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Error(e)
        }
    }
    
    fn failed(&self) -> bool {
        match self.data {
            TestResultData::Fail(..) => true,
            _ => false
        }
    }
}

struct TestResultFuture {
    rev_dep: RevDepName,
    rx: Receiver<TestResult>
}

impl TestResultFuture {
    fn take(self) -> TestResult {
        match self.rx.recv() {
            Ok(r) => r,
            Err(e) => {
                let r = RevDep {
                    name: self.rev_dep,
                    vers: Version::parse("0.0.0").unwrap()
                };
                TestResult::error(r, Error::from(e))
            }
        }
    }
}

fn new_result_future(rev_dep: RevDepName) -> (Sender<TestResult>, TestResultFuture) {
    let (tx, rx) = mpsc::channel();

    let fut = TestResultFuture {
        rev_dep: rev_dep,
        rx: rx
    };

    (tx, fut)
}

fn run_test(pool: &mut ThreadPool,
            base_crate: CrateOverride,
            next_crate: CrateOverride,
            rev_dep: RevDepName) -> TestResultFuture {
    let (result_tx, result_future) = new_result_future(rev_dep.clone());
    pool.execute(move || {
        let res = run_test_local(&base_crate, &next_crate, rev_dep);
        result_tx.send(res);
    });

    return result_future;
}

fn run_test_local(base_crate: &CrateOverride, next_crate: &CrateOverride, rev_dep: RevDepName) -> TestResult {

    // First, figure get the most recent version number
    let rev_dep = match resolve_rev_dep_version(rev_dep.clone()) {
        Ok(r) => r,
        Err(e) => {
            let rev_dep = RevDep {
                name: rev_dep,
                vers: Version::parse("0.0.0").unwrap()
            };
            return TestResult::error(rev_dep, e);
        }
    };

    debug!("testing rev dep: {:?}", rev_dep);

    // TODO: Decide whether the version of our crate requested by the
    // rev dep is semver-compatible with the in-development version.
    
    let base_result = match compile_with_custom_dep(&rev_dep, base_crate) {
        Ok(r) => r,
        Err(e) => return TestResult::error(rev_dep, e)
    };

    if base_result.failed() {
        return TestResult::broken(rev_dep, base_result);
    }
    let next_result = match compile_with_custom_dep(&rev_dep, next_crate) {
        Ok(r) => r,
        Err(e) => return TestResult::error(rev_dep, e)
    };

    if next_result.failed() {
        TestResult::fail(rev_dep, base_result, next_result)
    } else {
        TestResult::pass(rev_dep, base_result, next_result)
    }
}

fn resolve_rev_dep_version(name: RevDepName) -> Result<RevDep, Error> {
    debug!("resolving current version for {}", name);
    let ref url = crate_url(&name, None);
    let ref body = try!(http_get_to_string(url));
    // Download the crate info from crates.io
    let krate = try!(parse_crate(body));
    // Pull out the version numbers and sort them
    let versions = krate.versions.iter()
        .filter_map(|r| Version::parse(&*r.num).ok());
    let mut versions = versions.collect::<Vec<_>>();
    versions.sort();

    versions.pop().map(|v| {
        RevDep {
            name: name,
            vers: v
        }
    }).ok_or(Error::NoCrateVersions)
}

// The server returns much more info than this.
// This just defines pieces we need.
#[derive(RustcEncodable, RustcDecodable, Debug)]
struct RegistryCrate {
    versions: Vec<RegistryVersion>
}

#[derive(RustcEncodable, RustcDecodable, Debug)]
struct RegistryVersion {
    num: String
}

fn parse_crate(s: &str) -> Result<RegistryCrate, Error> {
    Ok(try!(json::decode(&s)))
}

#[derive(Debug, Clone)]
struct CompileResult {
    stdout: String,
    stderr: String,
    success: bool
}

impl CompileResult {
    fn failed(&self) -> bool {
        !self.success
    }
}

fn compile_with_custom_dep(rev_dep: &RevDep, krate: &CrateOverride) -> Result<CompileResult, Error> {
    let ref crate_handle = try!(get_crate_handle(rev_dep));
    let temp_dir = try!(TempDir::new("crusader"));
    let ref source_dir = temp_dir.path().join("source");
    try!(fs::create_dir(source_dir));
    try!(crate_handle.unpack_source_to(source_dir));

    match *krate {
        CrateOverride::Default => (),
        CrateOverride::Source(ref path) => {
            // Emit a .cargo/config file to override the project's
            // dependency on *our* project with the WIP.
            try!(emit_cargo_override_path(source_dir, path));
        }
    }

    // NB: The way cargo searches for .cargo/config, which we use to
    // override dependencies, depends on the CWD, and is not affacted
    // by the --manifest-path flag, so this is changing directories.
    let mut cmd = Command::new("cargo");
    let cmd = cmd.arg("build")
        .current_dir(source_dir);
    debug!("running cargo: {:?}", cmd);
    let r = try!(cmd.output());

    let success = r.status.success();

    debug!("result: {:?}", success);

    Ok(CompileResult {
        stdout: try!(String::from_utf8(r.stdout)),
        stderr: try!(String::from_utf8(r.stderr)),
        success: success
    })
}

struct CrateHandle(PathBuf);

fn get_crate_handle(rev_dep: &RevDep) -> Result<CrateHandle, Error> {
    let cache_path = Path::new("./.crusader/crate-cache");
    let ref crate_dir = cache_path.join(&rev_dep.name);
    try!(fs::create_dir_all(crate_dir));
    let crate_file = crate_dir.join(format!("{}-{}.crate", rev_dep.name, rev_dep.vers));
    // FIXME: Path::exists() is unstable so just opening the file
    let crate_file_exists = File::open(&crate_file).is_ok();
    if !crate_file_exists {
        let url = crate_url(&rev_dep.name,
                            Some(&format!("{}/download", rev_dep.vers)));
        let body = try!(http_get_bytes(&url));
        // FIXME: Should move this into place atomically
        let mut file = try!(File::create(&crate_file));
        try!(file.write_all(&body));
        try!(file.flush());
    }

    return Ok(CrateHandle(crate_file));
}

impl CrateHandle {
    fn unpack_source_to(&self, path: &Path) -> Result<(), Error> {
        debug!("unpackng {:?} to {:?}", self.0, path);
        let mut cmd = Command::new("tar");
        let cmd = cmd
            .arg("xzf")
            .arg(self.0.to_str().unwrap().to_owned())
            .arg("--strip-components=1")
            .arg("-C")
            .arg(path.to_str().unwrap().to_owned());
        let r = try!(cmd.output());
        if r.status.success() {
            Ok(())
        } else {
            // FIXME: Want to put r in this value but
            // process::Output doesn't implement Debug
            let s = String::from_utf8_lossy(&r.stderr).into_owned();
            Err(Error::ProcessError(s))
        }
    }
}

fn emit_cargo_override_path(source_dir: &Path, override_path: &Path) -> Result<(), Error> {
    debug!("overriding cargo path in {:?} with {:?}", source_dir, override_path);

    assert!(override_path.ends_with("Cargo.toml"));
    let override_path = override_path.parent().unwrap();

    // Since cargo is going to be run with --manifest-path to change
    // directories a relative path is not going to make sense.
    let override_path = if override_path.is_absolute() {
        override_path.to_path_buf()
    } else {
        try!(env::current_dir()).join(override_path)
    };
    let ref cargo_dir = source_dir.join(".cargo");
    try!(fs::create_dir_all(cargo_dir));
    let ref config_path = cargo_dir.join("config");
    let mut file = try!(File::create(config_path));
    let s = format!(r#"paths = ["{}"]"#, override_path.to_str().unwrap());
    try!(file.write_all(s.as_bytes()));
    try!(file.flush());

    Ok(())
}

fn report_results(res: Result<Vec<TestResult>, Error>) {
    println!("results: {:?}", res);
}

#[derive(Debug)]
enum Error {
    BadArgs,
    ManifestName(PathBuf),
    TestFailure(Vec<TestResult>),
    SemverError(semver::ParseError),
    TomlError(Vec<toml::ParserError>),
    IoError(io::Error),
    CurlError(curl::ErrCode),
    HttpError(CurlHttpResponseWrapper),
    Utf8Error(Utf8Error),
    JsonDecode(json::DecoderError),
    RecvError(RecvError),
    NoCrateVersions,
    FromUtf8Error(FromUtf8Error),
    ProcessError(String)
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

impl From<RecvError> for Error {
    fn from(e: RecvError) -> Error {
        Error::RecvError(e)
    }
}

impl From<FromUtf8Error> for Error {
    fn from(e: FromUtf8Error) -> Error {
        Error::FromUtf8Error(e)
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
