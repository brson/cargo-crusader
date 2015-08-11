// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

extern crate curl;
extern crate env_logger;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate rustc_serialize;
extern crate semver;
extern crate term;
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
use std::error::Error as StdError;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{PathBuf, Path};
use std::process::Command;
use std::str::Utf8Error;
use std::string::FromUtf8Error;
use std::sync::Mutex;
use std::sync::mpsc::{self, Sender, Receiver, RecvError};
use threadpool::ThreadPool;
use tempdir::TempDir;

fn main() {
    env_logger::init().unwrap();
    report_results(run());
}

fn run() -> Result<Vec<TestResult>, Error> {
    let config = try!(get_config());

    // Find all the crates on crates.io the depend on ours
    let rev_deps = try!(get_rev_deps(&config.crate_name));

    // Run all the tests in a thread pool and create a list of result
    // receivers.
    let mut result_rxs = Vec::new();
    let ref mut pool = ThreadPool::new(1);
    for rev_dep in rev_deps {
        let result = run_test(pool, config.clone(), rev_dep);
        result_rxs.push(result);
    }

    // Now wait for all the results and return them.
    let total = result_rxs.len();
    let results = result_rxs.into_iter().enumerate().map(|(i, r)| {
        let r = r.recv();
        report_quick_result(i + 1, total, &r);
        r
    });
    let results = results.collect::<Vec<_>>();

    Ok(results)
}

#[derive(Clone)]
struct Config {
    manifest_path: PathBuf,
    crate_name: String,
    base_override: CrateOverride,
    next_override: CrateOverride
}

#[derive(Clone)]
enum CrateOverride {
    Default,
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
        base_override: CrateOverride::Default,
        next_override: CrateOverride::Source(manifest)
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
    status(&format!("downloading reverse deps for {}", crate_name));
    let ref url = crate_url(crate_name, Some("reverse_dependencies"));
    let ref body = try!(http_get_to_string(url));
    let rev_deps = try!(parse_rev_deps(body));

    status(&format!("{} reverse deps", rev_deps.len()));

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
    Passed(CompileResult, CompileResult),
    Regressed(CompileResult, CompileResult),
    Broken(CompileResult),
    Error(Error),
}

impl TestResult {
    fn passed(rev_dep: RevDep, r1: CompileResult, r2: CompileResult) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Passed(r1, r2)
        }
    }

    fn regressed(rev_dep: RevDep, r1: CompileResult, r2: CompileResult) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Regressed(r1, r2)
        }
    }

    fn broken(rev_dep: RevDep, r: CompileResult) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Broken(r)
        }
    }

    fn error(rev_dep: RevDep, e: Error) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Error(e)
        }
    }
    
    fn quick_str(&self) -> &'static str {
        match self.data {
            TestResultData::Passed(..) => "passed",
            TestResultData::Regressed(..) => "regressed",
            TestResultData::Broken(_) => "broken",
            TestResultData::Error(_) => "error"
        }
    }

    fn html_class(&self) -> &'static str {
        self.quick_str()
    }

    fn html_anchor(&self) -> String {
        sanitize_link(&format!("{}-{}", self.rev_dep.name, self.rev_dep.vers))
    }
}

fn sanitize_link(s: &str) -> String {
    s.chars().map(|c| {
        let c = c.to_lowercase().collect::<Vec<_>>()[0];
        if c != '-' && (c < 'a' || c > 'z')
            && (c < '0' || c > '9') {
            '_'
        } else {
            c
        }
    }).collect()
}

struct TestResultReceiver {
    rev_dep: RevDepName,
    rx: Receiver<TestResult>
}

impl TestResultReceiver {
    fn recv(self) -> TestResult {
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

fn new_result_receiver(rev_dep: RevDepName) -> (Sender<TestResult>, TestResultReceiver) {
    let (tx, rx) = mpsc::channel();

    let fut = TestResultReceiver {
        rev_dep: rev_dep,
        rx: rx
    };

    (tx, fut)
}

fn run_test(pool: &mut ThreadPool,
            config: Config,
            rev_dep: RevDepName) -> TestResultReceiver {
    let (result_tx, result_rx) = new_result_receiver(rev_dep.clone());
    pool.execute(move || {
        let res = run_test_local(&config, rev_dep);
        result_tx.send(res).unwrap();
    });

    return result_rx;
}

fn run_test_local(config: &Config, rev_dep: RevDepName) -> TestResult {

    status(&format!("testing crate {}", rev_dep));

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

    // TODO: Decide whether the version of our crate requested by the
    // rev dep is semver-compatible with the in-development version.
    
    let base_result = match compile_with_custom_dep(&rev_dep, &config.base_override) {
        Ok(r) => r,
        Err(e) => return TestResult::error(rev_dep, e)
    };

    if base_result.failed() {
        return TestResult::broken(rev_dep, base_result);
    }
    let next_result = match compile_with_custom_dep(&rev_dep, &config.next_override) {
        Ok(r) => r,
        Err(e) => return TestResult::error(rev_dep, e)
    };

    if next_result.failed() {
        TestResult::regressed(rev_dep, base_result, next_result)
    } else {
        TestResult::passed(rev_dep, base_result, next_result)
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

fn status_lock<F>(f: F) where F: FnOnce() -> () {
   lazy_static! {
        static ref LOCK: Mutex<()> = Mutex::new(());
    }
    let _guard = LOCK.lock();
    f();
}

fn print_status_header() {
    print!("crusader: ");
}

fn print_color(s: &str, fg: term::color::Color) {
    if !really_print_color(s, fg) {
        print!("{}", s);
    }

    fn really_print_color(s: &str,
                          fg: term::color::Color) -> bool {
        if let Some(ref mut t) = term::stdout() {
            if t.fg(fg).is_err() { return false }
            let _ = t.attr(term::Attr::Bold);
            if write!(t, "{}", s).is_err() { return false }
            assert!(t.reset().unwrap());
        }

        true
    }
}

fn status(s: &str) {
    status_lock(|| {
        print_status_header();
        println!("{}", s);
    });
}

fn report_quick_result(current_num: usize, total: usize, result: &TestResult) {
    status_lock(|| {
        print_status_header();
        print!("result {} of {}, {} {}: ",
               current_num,
               total,
               result.rev_dep.name,
               result.rev_dep.vers
               );
        print_color(&format!("{}", result.quick_str()), result.into());
        println!("");
    });
}

fn report_results(res: Result<Vec<TestResult>, Error>) {
    match res {
        Ok(r) => {
            match export_report(r) {
                Ok((summary, report_path)) => report_success(summary, report_path),
                Err(e) => {
                    report_error(e)
                }
            }
        }
        Err(e) => {
            report_error(e)
        }
    }
}

fn report_error(e: Error) {
    println!("");
    print_color("error", term::color::BRIGHT_RED);
    println!(": {}", e.description());
    println!("");
    println!("{}", e);
    println!("");

    std::process::exit(-1);
}

fn export_report(mut results: Vec<TestResult>) -> Result<(Summary, PathBuf), Error> {
    let path = PathBuf::from("./crusader-report.html");
    let s = summarize_results(&results);

    results.sort_by(|a, b| {
        a.rev_dep.name.cmp(&b.rev_dep.name)
    });

    let ref mut file = try!(File::create(&path));
    try!(writeln!(file, "<!DOCTYPE html>"));

    try!(writeln!(file, "<head>"));
    try!(writeln!(file, "{}", r"

<style>
.passed { color: green; }
.regressed { color: red; }
.broken { color: yellow; }
.error { color: magenta; }
.stdout, .stderr, .test-exception-output { white-space: pre; }
</style>

"));
    try!(writeln!(file, "</head>"));

    try!(writeln!(file, "<body>"));

    // Print the summary table
    try!(writeln!(file, "<h1>Summary</h1>"));
    try!(writeln!(file, "<table>"));
    try!(writeln!(file, "<tr><th>Crate</th><th>Version</th><th>Result</th></tr>"));
    for result in &results {
        try!(writeln!(file, "<tr>"));
        try!(writeln!(file, "<td>"));
        try!(writeln!(file, "<a href='#{}'>", result.html_anchor()));
        try!(writeln!(file, "{}", result.rev_dep.name));
        try!(writeln!(file, "</a>"));
        try!(writeln!(file, "</td>"));
        try!(writeln!(file, "<td>{}</td>", result.rev_dep.vers));
        try!(writeln!(file, "<td class='{}'>{}</td>", result.html_class(), result.quick_str()));
        try!(writeln!(file, "</tr>"));
    }
    try!(writeln!(file, "</table>"));

    try!(writeln!(file, "<h1>Details</h1>"));
    for result in results {
        try!(writeln!(file, "<div class='complete-result'>"));
        try!(writeln!(file, "<a name='{}'></a>", result.html_anchor()));
        try!(writeln!(file, "<h2>"));
        try!(writeln!(file, "<span>{} {}</span>", result.rev_dep.name, result.rev_dep.vers));
        try!(writeln!(file, "<span class='{}'>{}</span>", result.html_class(), result.quick_str()));
        try!(writeln!(file, "</h2>"));
        match result.data {
            TestResultData::Passed(r1, r2) |
            TestResultData::Regressed(r1, r2) => {
                try!(export_compile_result(file, "before", r1));
                try!(export_compile_result(file, "after", r2));
            }
            TestResultData::Broken(r) => {
                try!(export_compile_result(file, "before", r));
            }
            TestResultData::Error(e) => {
                try!(export_error(file, e));
            }
        }
        try!(writeln!(file, "</div>"));
    }
    
    try!(writeln!(file, "</body>"));

    Ok((s, path))
}

fn export_compile_result(file: &mut File, label: &str, r: CompileResult) -> Result<(), Error> {
    let stdout = sanitize(&r.stdout);
    let stderr = sanitize(&r.stderr);
    try!(writeln!(file, "<h3>{}</h3>", label));
    try!(writeln!(file, "<div class='stdout'>\n{}\n</div>", stdout));
    try!(writeln!(file, "<div class='stderr'>\n{}\n</div>", stderr));

    Ok(())
}

fn export_error(file: &mut File, e: Error) -> Result<(), Error> {
    let err = sanitize(&format!("{}", e));
    try!(writeln!(file, "<h3>{}</h3>", "errors"));
    try!(writeln!(file, "<div class='test-exception-output'>\n{}\n</div>", err));

    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars().flat_map(|c| {
        match c {
            '<' => "&lt;".chars().collect(),
            '>' => "&gt;".chars().collect(),
            '&' => "&amp;".chars().collect(),
            _ => vec![c]
        }
    }).collect()
}

enum ReportResult { Passed, Regressed, Broken, Error }

impl Into<term::color::Color> for ReportResult {
    fn into(self) -> term::color::Color {
        match self {
            ReportResult::Passed => term::color::BRIGHT_GREEN,
            ReportResult::Regressed => term::color::BRIGHT_RED,
            ReportResult::Broken => term::color::BRIGHT_YELLOW,
            ReportResult::Error => term::color::BRIGHT_MAGENTA,
        }
    }
}

impl<'a> Into<term::color::Color> for &'a TestResult {
    fn into(self) -> term::color::Color {
        match self.data {
            TestResultData::Passed(..) => ReportResult::Passed,
            TestResultData::Regressed(..) => ReportResult::Regressed,
            TestResultData::Broken(_) => ReportResult::Broken,
            TestResultData::Error(_) => ReportResult::Error,
        }.into()
    }
}

fn report_success(s: Summary, p: PathBuf) {
    println!("");
    print!("passed: ");
    print_color(&format!("{}", s.passed), ReportResult::Passed.into());
    println!("");
    print!("regressed: ");
    print_color(&format!("{}", s.regressed), ReportResult::Regressed.into());
    println!("");
    print!("broken: ");
    print_color(&format!("{}", s.broken), ReportResult::Broken.into());
    println!("");
    print!("error: ");
    print_color(&format!("{}", s.error), ReportResult::Error.into());
    println!("");

    println!("");
    println!("full report: {}", p.to_str().unwrap());
    println!("");
    
    if s.regressed > 0 { std::process::exit(-2) }
}

#[derive(Default)]
struct Summary {
    broken: usize,
    regressed: usize,
    passed: usize,
    error: usize
}

fn summarize_results(results: &[TestResult]) -> Summary {
    let mut sum = Summary::default();

    for result in results {
        match result.data {
            TestResultData::Broken(..) => sum.broken += 1,
            TestResultData::Regressed(..) => sum.regressed += 1,
            TestResultData::Passed(..) => sum.passed += 1,
            TestResultData::Error(..) => sum.error += 1,
        }
    }

    return sum;
}

#[derive(Debug)]
enum Error {
    ManifestName(PathBuf),
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

macro_rules! convert_error {
    ($from:ty, $to:ident) => (
        impl From<$from> for Error {
            fn from(e: $from) -> Error {
                Error::$to(e)
            }
        }
    )
}

convert_error!(semver::ParseError, SemverError);
convert_error!(io::Error, IoError);
convert_error!(curl::ErrCode, CurlError);
convert_error!(Utf8Error, Utf8Error);
convert_error!(json::DecoderError, JsonDecode);
convert_error!(RecvError, RecvError);
convert_error!(FromUtf8Error, FromUtf8Error);

struct CurlHttpResponseWrapper(CurlHttpResponse);

impl fmt::Debug for CurlHttpResponseWrapper {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let CurlHttpResponseWrapper(ref resp) = *self;
        let tup = (resp.get_code(), resp.get_headers(), resp.get_body());
        try!(fmt.write_str(&format!("{:?}", tup)));

        Ok(())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self {
            Error::ManifestName(_) => fmt.write_str(self.description()),
            Error::SemverError(ref e) => e.fmt(fmt),
            Error::TomlError(_) => fmt.write_str(self.description()),
            Error::IoError(ref e) => e.fmt(fmt),
            Error::CurlError(c) => c.fmt(fmt),
            Error::HttpError(CurlHttpResponseWrapper(ref r)) => r.fmt(fmt),
            Error::Utf8Error(ref e) => e.fmt(fmt),
            Error::JsonDecode(ref e) => e.fmt(fmt),
            Error::RecvError(ref e) => e.fmt(fmt),
            Error::NoCrateVersions => fmt.write_str(self.description()),
            Error::FromUtf8Error(ref e) => e.fmt(fmt),
            Error::ProcessError(ref s) => s.fmt(fmt)
        }
    }
}

impl StdError for Error {
    fn description(&self) -> &str {
        match *self {
            Error::ManifestName(_) => "error extracting crate name from manifest",
            Error::SemverError(ref e) => e.description(),
            Error::TomlError(_) => "error parsing TOML",
            Error::IoError(ref e) => e.description(),
            Error::CurlError(_) => "curl error",
            Error::HttpError(_) => "HTTP error",
            Error::Utf8Error(ref e) => e.description(),
            Error::JsonDecode(ref e) => e.description(),
            Error::RecvError(ref e) => e.description(),
            Error::NoCrateVersions => "crate has no published versions",
            Error::FromUtf8Error(ref e) => e.description(),
            Error::ProcessError(_) => "unexpected subprocess failure"
        }
    }
}
