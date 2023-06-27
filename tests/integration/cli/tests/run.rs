//! Basic tests for the `run` subcommand

use std::{
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use assert_cmd::{assert::Assert, prelude::OutputAssertExt};
use once_cell::sync::Lazy;
use predicates::str::contains;
use rand::Rng;
use reqwest::{blocking::Client, IntoUrl};
use tempfile::TempDir;
use wasmer_integration_tests_cli::{asset_path, c_asset_path, get_wasmer_path};

const HTTP_GET_TIMEOUT: Duration = Duration::from_secs(5);

static RUST_LOG: Lazy<String> = Lazy::new(|| {
    if cfg!(feature = "debug") {
        "trace".to_string()
    } else {
        [
            "info",
            "wasmer_wasix::resolve=debug",
            "wasmer_wasix::runners=debug",
            "wasmer_wasix=debug",
            "virtual_fs::trace_fs=trace",
        ]
        .join(",")
    }
});

fn wasi_test_python_path() -> PathBuf {
    c_asset_path().join("python-0.1.0.wasmer")
}

fn wasi_test_wasm_path() -> PathBuf {
    c_asset_path().join("qjs.wasm")
}

fn test_no_imports_wat_path() -> PathBuf {
    asset_path().join("fib.wat")
}

fn test_no_start_wat_path() -> PathBuf {
    asset_path().join("no_start.wat")
}

/// Ignored on Windows because running vendored packages does not work
/// since Windows does not allow `::` characters in filenames (every other OS does)
///
/// The syntax for vendored package atoms has to be reworked for this to be fixed, see
/// https://github.com/wasmerio/wasmer/issues/3535
// FIXME: Re-enable. See https://github.com/wasmerio/wasmer/issues/3717
#[test]
#[ignore]
fn test_run_customlambda() {
    let assert = Command::new(get_wasmer_path())
        .arg("config")
        .arg("--bindir")
        .assert()
        .success();
    let bindir = std::str::from_utf8(&assert.get_output().stdout)
        .expect("wasmer config --bindir stdout failed");

    // /Users/fs/.wasmer/bin
    let checkouts_path = Path::new(bindir.trim())
        .parent()
        .expect("--bindir: no parent")
        .join("checkouts");
    println!("checkouts path: {}", checkouts_path.display());
    let _ = std::fs::remove_dir_all(&checkouts_path);

    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("https://wapm.io/ciuser/customlambda")
        // TODO: this argument should not be necessary later
        // see https://github.com/wasmerio/wasmer/issues/3514
        .arg("customlambda.py")
        .arg("55")
        .assert()
        .success();
    assert.stdout("139583862445\n");

    // Run again to verify the caching
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("https://wapm.io/ciuser/customlambda")
        // TODO: this argument should not be necessary later
        // see https://github.com/wasmerio/wasmer/issues/3514
        .arg("customlambda.py")
        .arg("55")
        .assert()
        .success();
    assert.stdout("139583862445\n");
}

fn assert_tarball_is_present_local(target: &str) -> Result<PathBuf, anyhow::Error> {
    let wasmer_dir = std::env::var("WASMER_DIR").expect("no WASMER_DIR set");
    let directory = match target {
        "aarch64-darwin" => "wasmer-darwin-arm64.tar.gz",
        "x86_64-darwin" => "wasmer-darwin-amd64.tar.gz",
        "x86_64-linux-gnu" => "wasmer-linux-amd64.tar.gz",
        "aarch64-linux-gnu" => "wasmer-linux-aarch64.tar.gz",
        "x86_64-windows-gnu" => "wasmer-windows-gnu64.tar.gz",
        _ => return Err(anyhow::anyhow!("unknown target {target}")),
    };
    let libwasmer_cache_path = Path::new(&wasmer_dir).join("cache").join(directory);
    if !libwasmer_cache_path.exists() {
        return Err(anyhow::anyhow!(
            "targz {} does not exist",
            libwasmer_cache_path.display()
        ));
    }
    println!("using targz {}", libwasmer_cache_path.display());
    Ok(libwasmer_cache_path)
}

// FIXME: Fix and re-enable this test
// See https://github.com/wasmerio/wasmer/issues/3615
#[test]
#[ignore]
fn test_cross_compile_python_windows() {
    let temp_dir = tempfile::TempDir::new().unwrap();

    #[cfg(not(windows))]
    let targets = &[
        "aarch64-darwin",
        "x86_64-darwin",
        "x86_64-linux-gnu",
        "aarch64-linux-gnu",
        "x86_64-windows-gnu",
    ];

    #[cfg(windows)]
    let targets = &[
        "aarch64-darwin",
        "x86_64-darwin",
        "x86_64-linux-gnu",
        "aarch64-linux-gnu",
    ];

    // MUSL has no support for LLVM in C-API
    #[cfg(target_env = "musl")]
    let compilers = &["cranelift", "singlepass"];
    #[cfg(not(target_env = "musl"))]
    let compilers = &["cranelift", "singlepass", "llvm"];

    // llvm-objdump  --disassemble-all --demangle ./objects/wasmer_vm-50cb118b098c15db.wasmer_vm.60425a0a-cgu.12.rcgu.o
    // llvm-objdump --macho --exports-trie ~/.wasmer/cache/wasmer-darwin-arm64/lib/libwasmer.dylib
    let excluded_combinations = &[
        ("aarch64-darwin", "llvm"), // LLVM: aarch64 not supported relocation Arm64MovwG0 not supported
        ("aarch64-linux-gnu", "llvm"), // LLVM: aarch64 not supported relocation Arm64MovwG0 not supported
        // https://github.com/ziglang/zig/issues/13729
        ("x86_64-darwin", "llvm"), // undefined reference to symbol 'wasmer_vm_raise_trap' kind Unknown
        ("x86_64-windows-gnu", "llvm"), // unimplemented symbol `wasmer_vm_raise_trap` kind Unknown
    ];

    for t in targets {
        for c in compilers {
            if excluded_combinations.contains(&(t, c)) {
                continue;
            }
            println!("{t} target {c}");
            let python_wasmer_path = temp_dir.path().join(format!("{t}-python"));

            let tarball = match std::env::var("GITHUB_TOKEN") {
                Ok(_) => Some(assert_tarball_is_present_local(t).unwrap()),
                Err(_) => None,
            };
            let mut cmd = Command::new(get_wasmer_path());

            cmd.arg("create-exe");
            cmd.arg(wasi_test_python_path());
            cmd.arg("--target");
            cmd.arg(t);
            cmd.arg("-o");
            cmd.arg(python_wasmer_path.clone());
            cmd.arg(format!("--{c}"));
            if std::env::var("GITHUB_TOKEN").is_ok() {
                cmd.arg("--debug-dir");
                cmd.arg(format!("{t}-{c}"));
            }

            if t.contains("x86_64") && *c == "singlepass" {
                cmd.arg("-m");
                cmd.arg("avx");
            }

            if let Some(t) = tarball {
                cmd.arg("--tarball");
                cmd.arg(t);
            }

            let assert = cmd.assert().success();

            if !python_wasmer_path.exists() {
                let p = std::fs::read_dir(temp_dir.path())
                    .unwrap()
                    .filter_map(|e| Some(e.ok()?.path()))
                    .collect::<Vec<_>>();
                let output = assert.get_output();
                panic!("target {t} was not compiled correctly tempdir: {p:#?}, {output:?}",);
            }
        }
    }
}

#[test]
fn run_whoami_works() {
    // running test locally: should always pass since
    // developers don't have access to WAPM_DEV_TOKEN
    if std::env::var("GITHUB_TOKEN").is_err() {
        return;
    }

    let ciuser_token = std::env::var("WAPM_DEV_TOKEN").expect("no CIUSER / WAPM_DEV_TOKEN token");
    // Special case: GitHub secrets aren't visible to outside collaborators
    if ciuser_token.is_empty() {
        return;
    }

    let assert = Command::new(get_wasmer_path())
        .arg("whoami")
        .arg("--registry=wapm.dev")
        .arg("--token")
        .arg(&ciuser_token)
        .assert()
        .success();

    assert
        .stdout("logged into registry \"https://registry.wapm.dev/graphql\" as user \"ciuser\"\n");
}

#[test]
fn run_wasi_works() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(wasi_test_wasm_path())
        .arg("--")
        .arg("-e")
        .arg("print(3 * (4 + 5))")
        .assert()
        .success();

    assert.stdout("27\n");
}

// FIXME: Re-enable. See https://github.com/wasmerio/wasmer/issues/3717
#[test]
#[ignore]
fn test_wasmer_run_pirita_works() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let python_wasmer_path = temp_dir.path().join("python.wasmer");
    std::fs::copy(wasi_test_python_path(), &python_wasmer_path).unwrap();

    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(python_wasmer_path)
        .arg("--")
        .arg("-c")
        .arg("print(\"hello\")")
        .assert()
        .success();

    assert.stdout("hello\n");
}

// FIXME: Re-enable. See https://github.com/wasmerio/wasmer/issues/3717
#[test]
#[ignore]
fn test_wasmer_run_pirita_url_works() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("https://wapm.dev/syrusakbary/python")
        .arg("--")
        .arg("-c")
        .arg("print(\"hello\")")
        .assert()
        .success();

    assert.stdout("hello\n");
}

#[test]
fn test_wasmer_run_works_with_dir() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let qjs_path = temp_dir.path().join("qjs.wasm");

    std::fs::copy(wasi_test_wasm_path(), &qjs_path).unwrap();
    std::fs::copy(
        c_asset_path().join("qjs-wasmer.toml"),
        temp_dir.path().join("wasmer.toml"),
    )
    .unwrap();

    assert!(temp_dir.path().exists());
    assert!(temp_dir.path().join("wasmer.toml").exists());
    assert!(temp_dir.path().join("qjs.wasm").exists());

    // test with "wasmer qjs.wasm"
    Command::new(get_wasmer_path())
        .arg(temp_dir.path())
        .arg("--")
        .arg("--quit")
        .assert()
        .success();

    // test again with "wasmer run qjs.wasm"
    Command::new(get_wasmer_path())
        .arg("run")
        .arg(temp_dir.path())
        .arg("--")
        .arg("--quit")
        .assert()
        .success();
}

// FIXME: Re-enable. See https://github.com/wasmerio/wasmer/issues/3717
#[ignore]
#[cfg_attr(target_env = "musl", ignore)]
#[test]
fn test_wasmer_run_works() {
    let assert = Command::new(get_wasmer_path())
        .arg("https://wapm.io/python/python")
        .arg(format!("--mapdir=.:{}", asset_path().display()))
        .arg("test.py")
        .assert()
        .success();

    assert.stdout("hello\n");

    // same test again, but this time with "wasmer run ..."
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("https://wapm.io/python/python")
        .arg(format!("--mapdir=.:{}", asset_path().display()))
        .arg("test.py")
        .assert()
        .success();

    assert.stdout("hello\n");

    // same test again, but this time without specifying the registry in the URL
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("python/python")
        .arg(format!("--mapdir=.:{}", asset_path().display()))
        .arg("--registry=wasmer.io")
        .arg("test.py")
        .assert()
        .success();

    assert.stdout("hello\n");

    // same test again, but this time with only the command "python" (should be looked up locally)
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("_/python")
        .arg(format!("--mapdir=.:{}", asset_path().display()))
        .arg("--registry=wasmer.io")
        .arg("test.py")
        .assert()
        .success();

    assert.stdout("hello\n");
}

#[test]
fn run_no_imports_wasm_works() {
    Command::new(get_wasmer_path())
        .arg("run")
        .arg(test_no_imports_wat_path())
        .assert()
        .success();
}

#[test]
fn run_wasi_works_non_existent() -> anyhow::Result<()> {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("does-not/exist")
        .assert()
        .failure();

    assert
        .stderr(contains(
            "Unable to find \"does-not/exist\" in the registry",
        ))
        .stderr(contains("1: Not found"));

    Ok(())
}

// FIXME: Re-enable. See https://github.com/wasmerio/wasmer/issues/3717
#[ignore]
#[test]
fn run_test_caching_works_for_packages() {
    // we're testing the cache, so we don't want to reuse the current user's
    // $WASMER_DIR
    let wasmer_dir = TempDir::new().unwrap();
    let rust_log = [
        "wasmer_wasix::runtime::resolver::wapm_source=debug",
        "wasmer_wasix::runtime::package_loader::builtin_loader=debug",
    ]
    .join(",");

    let assert = Command::new(get_wasmer_path())
        .arg("python/python")
        .arg(format!("--mapdir=.:{}", asset_path().display()))
        .arg("--registry=wasmer.io")
        .arg("test.py")
        .env("WASMER_DIR", wasmer_dir.path())
        .env("RUST_LOG", &rust_log)
        .assert();

    assert
        .success()
        .stderr("Downloading a webc")
        .stderr("Querying the GraphQL API");

    let assert = Command::new(get_wasmer_path())
        .arg("python/python")
        .arg(format!("--mapdir=.:{}", asset_path().display()))
        .arg("--registry=wasmer.io")
        .arg("test.py")
        .env("WASMER_DIR", wasmer_dir.path())
        .env("RUST_LOG", &rust_log)
        .assert();

    assert.success().stderr("asdf");
}

#[test]
fn run_test_caching_works_for_packages_with_versions() {
    let assert = Command::new(get_wasmer_path())
        .arg("python/python@0.1.0")
        .arg(format!("--mapdir=/app:{}", asset_path().display()))
        .arg("--registry=wasmer.io")
        .arg("/app/test.py")
        .assert()
        .success();

    assert.stdout("hello\n");

    let assert = Command::new(get_wasmer_path())
        .arg("python/python@0.1.0")
        .arg(format!("--mapdir=/app:{}", asset_path().display()))
        .arg("--registry=wasmer.io")
        .arg("/app/test.py")
        .env(
            "RUST_LOG",
            "wasmer_wasix::runtime::package_loader::builtin_loader=debug",
        )
        .assert();

    assert
        .success()
        // it should have ran like normal
        .stdout("hello\n")
        // we hit the cache while fetching the package
        .stderr(contains(
            "builtin_loader: Cache hit! pkg.name=\"python\" pkg.version=0.1.0",
        ));
}

// FIXME: Re-enable. See https://github.com/wasmerio/wasmer/issues/3717
#[ignore]
#[test]
fn run_test_caching_works_for_urls() {
    let assert = Command::new(get_wasmer_path())
        .arg("https://wapm.io/python/python")
        .arg(format!("--mapdir=.:{}", asset_path().display()))
        .arg("test.py")
        .assert()
        .success();

    assert.stdout("hello\n");

    let time = std::time::Instant::now();

    let assert = Command::new(get_wasmer_path())
        .arg("https://wapm.io/python/python")
        .arg(format!("--mapdir=.:{}", asset_path().display()))
        .arg("test.py")
        .assert()
        .success();

    assert.stdout("hello\n");

    // package should be cached
    assert!(std::time::Instant::now() - time < std::time::Duration::from_secs(1));
}

// This test verifies that "wasmer run --invoke _start module.wat"
// works the same as "wasmer run module.wat" (without --invoke).
#[test]
fn run_invoke_works_with_nomain_wasi() {
    // In this example the function "wasi_unstable.arg_sizes_get"
    // is a function that is imported from the WASI env.
    let wasi_wat = "
    (module
        (import \"wasi_unstable\" \"args_sizes_get\"
          (func $__wasi_args_sizes_get (param i32 i32) (result i32)))
        (func $_start)
        (memory 1)
        (export \"memory\" (memory 0))
        (export \"_start\" (func $_start))
      )
    ";

    let random = rand::random::<u64>();
    let module_file = std::env::temp_dir().join(format!("{random}.wat"));
    std::fs::write(&module_file, wasi_wat.as_bytes()).unwrap();

    Command::new(get_wasmer_path())
        .arg("run")
        .arg(&module_file)
        .assert()
        .success();

    Command::new(get_wasmer_path())
        .arg("run")
        .arg("--invoke")
        .arg("_start")
        .arg(&module_file)
        .assert()
        .success();

    std::fs::remove_file(&module_file).unwrap();
}

#[test]
fn run_no_start_wasm_report_error() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(test_no_start_wat_path())
        .assert()
        .failure();

    assert.stderr(contains("The module doesn't contain a \"_start\" function"));
}

// Test that wasmer can run a complex path
#[test]
fn test_wasmer_run_complex_url() {
    let wasm_test_path = wasi_test_wasm_path();
    let wasm_test_path = wasm_test_path.canonicalize().unwrap_or(wasm_test_path);
    let mut wasm_test_path = format!("{}", wasm_test_path.display());
    if wasm_test_path.starts_with(r#"\\?\"#) {
        wasm_test_path = wasm_test_path.replacen(r#"\\?\"#, "", 1);
    }
    #[cfg(target_os = "windows")]
    {
        wasm_test_path = wasm_test_path.replace("D:\\", "D://");
        wasm_test_path = wasm_test_path.replace("C:\\", "C://");
        wasm_test_path = wasm_test_path.replace("c:\\", "c://");
        wasm_test_path = wasm_test_path.replace("\\", "/");
        // wasmer run used to fail on c:\Users\username\wapm_packages\ ...
        assert!(
            wasm_test_path.contains("://"),
            "wasm_test_path path is not complex enough"
        );
    }

    Command::new(get_wasmer_path())
        .arg("run")
        .arg(wasm_test_path)
        .arg("--")
        .arg("-q")
        .assert()
        .success();
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn wasi_runner_on_disk() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::qjs())
        .arg("--")
        .arg("--eval")
        .arg("console.log('Hello, World!')")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

/// See <https://github.com/wasmerio/wasmer/issues/4010> for more.
#[test]
fn wasi_runner_on_disk_mount_using_relative_directory_on_the_host() {
    let temp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    std::fs::write(temp.path().join("main.py"), "print('Hello, World!')").unwrap();

    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::python())
        .arg("--mapdir=/app:.")
        .arg("--")
        .arg("/app/main.py")
        .env("RUST_LOG", &*RUST_LOG)
        .current_dir(temp.path())
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn wasi_runner_on_disk_with_mounted_directories() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("index.js"), "console.log('Hello, World!')").unwrap();

    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::qjs())
        .arg(format!("--mapdir=/app:{}", temp.path().display()))
        .arg("--")
        .arg("/app/index.js")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn wasi_runner_on_disk_with_mounted_directories_and_webc_volumes() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("main.py"), "print('Hello, World!')").unwrap();

    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::python())
        .arg(format!("--mapdir=/app:{}", temp.path().display()))
        .arg("--")
        .arg("-B")
        .arg("/app/main.py")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn wasi_runner_on_disk_with_dependencies() {
    let port = random_port();
    let mut cmd = Command::new(get_wasmer_path());
    cmd.arg("run")
        .arg(fixtures::hello())
        .arg(format!("--env=SERVER_PORT={port}"))
        .arg("--net")
        .arg("--")
        .arg("--log-level=info")
        .env("RUST_LOG", &*RUST_LOG);
    let mut child = JoinableChild::spawn(cmd);
    child.wait_for_stderr("listening");

    // Make sure we get the page we want
    let html = reqwest::blocking::get(format!("http://localhost:{port}/"))
        .unwrap()
        .text()
        .unwrap();
    assert!(html.contains("<title>Hello World</title>"), "{html}");

    // and make sure our request was logged
    child
        .join()
        .stderr(contains("incoming request: method=GET uri=/"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn webc_files_on_disk_with_multiple_commands_require_an_entrypoint_flag() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::wabt())
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    let msg = r#"Unable to determine the WEBC file's entrypoint. Please choose one of ["wasm-interp", "wasm-strip", "wasm-validate", "wasm2wat", "wast2json", "wat2wasm"]"#;
    assert.failure().stderr(contains(msg));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn wasi_runner_on_disk_with_env_vars() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::python())
        .arg("--env=SOME_VAR=Hello, World!")
        .arg("--")
        .arg("-B")
        .arg("-c")
        .arg("import os; print(os.environ['SOME_VAR'])")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn wcgi_runner_on_disk() {
    // Start the WCGI server in the background
    let port = random_port();
    let mut cmd = Command::new(get_wasmer_path());
    cmd.arg("run")
        .arg(format!("--addr=127.0.0.1:{port}"))
        .arg(fixtures::static_server())
        .env("RUST_LOG", &*RUST_LOG);

    // Let's run the command and wait until the server has started
    let mut child = JoinableChild::spawn(cmd);
    child.wait_for_stdout("WCGI Server running");

    // make the request
    let body = http_get(format!("http://127.0.0.1:{port}/")).unwrap();
    assert!(body.contains("<title>Index of /</title>"), "{body}");

    // Let's make sure 404s work too
    let err = http_get(format!("http://127.0.0.1:{port}/this/does/not/exist.html")).unwrap_err();
    assert_eq!(err.status().unwrap(), reqwest::StatusCode::NOT_FOUND);

    // And kill the server, making sure it generated the expected logs
    let assert = child.join();

    assert
        .stderr(contains("Starting the server"))
        .stderr(contains(
            "response generated method=GET uri=/ status_code=200 OK",
        ))
        .stderr(contains(
            "response generated method=GET uri=/this/does/not/exist.html status_code=404 Not Found",
        ));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn wcgi_runner_on_disk_with_mounted_directories() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("file.txt"), "Hello, World!").unwrap();
    // Start the WCGI server in the background
    let port = random_port();
    let mut cmd = Command::new(get_wasmer_path());
    cmd.arg("run")
        .arg(format!("--addr=127.0.0.1:{port}"))
        .arg(format!("--mapdir=/path/to:{}", temp.path().display()))
        .arg(fixtures::static_server())
        .env("RUST_LOG", &*RUST_LOG);

    // Let's run the command and wait until the server has started
    let mut child = JoinableChild::spawn(cmd);
    child.wait_for_stdout("WCGI Server running");

    let body = http_get(format!("http://127.0.0.1:{port}/path/to/file.txt")).unwrap();
    assert!(body.contains("Hello, World!"), "{body}");

    // And kill the server, making sure it generated the expected logs
    let assert = child.join();

    assert
        .stderr(contains("Starting the server"))
        .stderr(contains(
            "response generated method=GET uri=/path/to/file.txt status_code=200 OK",
        ));
}

/// See https://github.com/wasmerio/wasmer/issues/3794
#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn issue_3794_unable_to_mount_relative_paths() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("message.txt"), b"Hello, World!").unwrap();

    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::coreutils())
        .arg(format!("--mapdir=./some-dir/:{}", temp.path().display()))
        .arg("--command-name=cat")
        .arg("--")
        .arg("./some-dir/message.txt")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
#[cfg_attr(
    windows,
    ignore = "FIXME(Michael-F-Bryan): Temporarily broken on Windows - https://github.com/wasmerio/wasmer/issues/3929"
)]
fn merged_filesystem_contains_all_files() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::bash())
        .arg("--entrypoint=bash")
        .arg("--use")
        .arg(fixtures::coreutils())
        .arg("--use")
        .arg(fixtures::python())
        .arg("--")
        .arg("-c")
        .arg("ls -l /usr/coreutils/*.md && ls -l /lib/python3.6/*.py")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert
        .success()
        .stdout(contains("/usr/coreutils/README.md"))
        .stdout(contains("/lib/python3.6/this.py"));
}

#[test]
fn run_a_wasi_executable() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::qjs())
        .arg("--")
        .arg("--eval")
        .arg("console.log('Hello, World!')")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
fn wasm_file_with_no_abi() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::fib())
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success();
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn error_if_no_start_function_found() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(fixtures::wat_no_start())
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert
        .failure()
        .stderr(contains("The module doesn't contain a \"_start\" function"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn run_a_pre_compiled_wasm_file() {
    let temp = TempDir::new().unwrap();
    let dest = temp.path().join("qjs.wasmu");
    let qjs = fixtures::qjs();
    // Make sure it is compiled
    Command::new(get_wasmer_path())
        .arg("compile")
        .arg("-o")
        .arg(&dest)
        .arg(&qjs)
        .assert()
        .success();
    assert!(dest.exists());

    // Now we can try to run the compiled artifact
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(&dest)
        .arg("--")
        .arg("--eval")
        .arg("console.log('Hello, World!')")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn wasmer_run_some_directory() {
    let temp = TempDir::new().unwrap();
    std::fs::copy(fixtures::qjs(), temp.path().join("qjs.wasm")).unwrap();
    std::fs::copy(fixtures::qjs_wasmer_toml(), temp.path().join("wasmer.toml")).unwrap();

    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg(temp.path())
        .arg("--")
        .arg("--eval")
        .arg("console.log('Hello, World!')")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn run_quickjs_via_package_name() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("saghul/quickjs")
        .arg("--entrypoint=quickjs")
        .arg("--registry=wapm.io")
        .arg("--")
        .arg("--eval")
        .arg("console.log('Hello, World!')")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
fn run_quickjs_via_url() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("https://wapm.io/saghul/quickjs")
        .arg("--entrypoint=quickjs")
        .arg("--")
        .arg("--eval")
        .arg("console.log('Hello, World!')")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    assert.success().stdout(contains("Hello, World!"));
}

#[test]
#[cfg_attr(
    all(target_env = "musl", target_os = "linux"),
    ignore = "wasmer run-unstable segfaults on musl"
)]
#[cfg_attr(
    windows,
    ignore = "TODO(Michael-F-Bryan): Figure out why WasiFs::get_inode_at_path_inner() returns Errno::notcapable on Windows"
)]
fn run_bash_using_coreutils() {
    let assert = Command::new(get_wasmer_path())
        .arg("run")
        .arg("sharrattj/bash")
        .arg("--entrypoint=bash")
        .arg("--use=sharrattj/coreutils")
        .arg("--registry=wapm.io")
        .arg("--")
        .arg("-c")
        .arg("ls /bin")
        .env("RUST_LOG", &*RUST_LOG)
        .assert();

    // Note: the resulting filesystem should contain the main command as
    // well as the commands from all the --use packages

    let some_expected_binaries = [
        "arch", "base32", "base64", "baseenc", "basename", "bash", "cat",
    ]
    .join("\n");
    assert.success().stdout(contains(some_expected_binaries));
}

mod fixtures {
    use std::path::{Path, PathBuf};

    use wasmer_integration_tests_cli::{asset_path, c_asset_path};

    /// A WEBC file containing the Python interpreter, compiled to WASI.
    pub fn python() -> PathBuf {
        c_asset_path().join("python-0.1.0.wasmer")
    }

    pub fn coreutils() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("webc")
            .join("coreutils-1.0.16-e27dbb4f-2ef2-4b44-b46a-ddd86497c6d7.webc")
    }

    pub fn bash() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("webc")
            .join("bash-1.0.16-f097441a-a80b-4e0d-87d7-684918ef4bb6.webc")
    }

    /// A WEBC file containing `wat2wasm`, `wasm-validate`, and other helpful
    /// WebAssembly-related commands.
    pub fn wabt() -> PathBuf {
        c_asset_path().join("wabt-1.0.37.wasmer")
    }

    /// A WEBC file containing the WCGI static server.
    pub fn static_server() -> PathBuf {
        c_asset_path().join("staticserver.webc")
    }

    /// The QuickJS interpreter, compiled to a WASI module.
    pub fn qjs() -> PathBuf {
        c_asset_path().join("qjs.wasm")
    }

    pub fn hello() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("webc")
            .join("hello-0.1.0-665d2ddc-80e6-4845-85d3-4587b1693bb7.webc")
    }

    /// The `wasmer.toml` file for QuickJS.
    pub fn qjs_wasmer_toml() -> PathBuf {
        c_asset_path().join("qjs-wasmer.toml")
    }

    /// An executable which calculates fib(40) and exits with no output.
    pub fn fib() -> PathBuf {
        asset_path().join("fib.wat")
    }

    pub fn wat_no_start() -> PathBuf {
        asset_path().join("no_start.wat")
    }
}

/// A helper that wraps [`Child`] to make sure it gets terminated
/// when it is no longer needed.
struct JoinableChild {
    command: Command,
    child: Option<Child>,
}

impl JoinableChild {
    fn spawn(mut cmd: Command) -> Self {
        let child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        JoinableChild {
            child: Some(child),
            command: cmd,
        }
    }

    /// Keep reading lines from the child's stdout until a line containing the
    /// desired text is found.
    fn wait_for_stdout(&mut self, text: &str) -> String {
        let stdout = self
            .child
            .as_mut()
            .and_then(|child| child.stdout.as_mut())
            .unwrap();

        wait_for(text, stdout)
    }

    /// Keep reading lines from the child's stderr until a line containing the
    /// desired text is found.
    fn wait_for_stderr(&mut self, text: &str) -> String {
        let stderr = self
            .child
            .as_mut()
            .and_then(|child| child.stderr.as_mut())
            .unwrap();

        wait_for(text, stderr)
    }

    /// Kill the underlying [`Child`] and get an [`Assert`] we
    /// can use to check it.
    fn join(mut self) -> Assert {
        let mut child = self.child.take().unwrap();
        child.kill().unwrap();
        child.wait_with_output().unwrap().assert()
    }
}

fn wait_for(text: &str, reader: &mut dyn Read) -> String {
    let mut all_output = String::new();

    loop {
        let line = read_line(reader).unwrap();

        if line.is_empty() {
            eprintln!("=== All Output === ");
            eprintln!("{all_output}");
            panic!("EOF before \"{text}\" was found");
        }

        let found = line.contains(text);
        all_output.push_str(&line);

        if found {
            return all_output;
        }
    }
}

fn read_line(reader: &mut dyn Read) -> Result<String, std::io::Error> {
    let mut line = Vec::new();

    while !line.ends_with(&[b'\n']) {
        let mut buffer = [0_u8];
        match reader.read_exact(&mut buffer) {
            Ok(_) => {
                line.push(buffer[0]);
            }
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
    }

    let line = String::from_utf8(line).map_err(|e| std::io::Error::new(ErrorKind::Other, e))?;
    Ok(line)
}

impl Drop for JoinableChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            eprintln!("==== WARNING: Child was dropped before being joined ====");
            eprintln!("Command: {:?}", self.command);

            let _ = child.kill();

            if let Some(mut stderr) = child.stderr.take() {
                let mut buffer = String::new();
                if stderr.read_to_string(&mut buffer).is_ok() {
                    eprintln!("---- STDERR ----");
                    eprintln!("{buffer}");
                }
            }

            if let Some(mut stdout) = child.stdout.take() {
                let mut buffer = String::new();
                if stdout.read_to_string(&mut buffer).is_ok() {
                    eprintln!("---- STDOUT ----");
                    eprintln!("{buffer}");
                }
            }

            if !std::thread::panicking() {
                panic!("Child was dropped before being joined");
            }
        }
    }
}

/// Send a GET request to a particular URL, automatically retrying (with
/// a timeout) if there are any connection errors.
fn http_get(url: impl IntoUrl) -> Result<String, reqwest::Error> {
    let start = Instant::now();
    let url = url.into_url().unwrap();

    let client = Client::new();

    while start.elapsed() < HTTP_GET_TIMEOUT {
        match client.get(url.clone()).send() {
            Ok(response) => {
                return response.error_for_status()?.text();
            }
            Err(e) if e.is_connect() => continue,
            Err(other) => return Err(other),
        }
    }

    panic!("Didn't receive a response from \"{url}\" within the allocated time");
}

fn random_port() -> u16 {
    rand::thread_rng().gen_range(10_000_u16..u16::MAX)
}
