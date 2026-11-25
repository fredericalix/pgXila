//! Compiles the C side of the connection guard against the PostgreSQL headers.

use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/tls_guard.c");
    println!("cargo:rerun-if-env-changed=PGRX_PG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PG_CONFIG");

    // The same pg_config that pgrx builds against, when it tells us.
    let pg_config = env::var("PGRX_PG_CONFIG_PATH")
        .or_else(|_| env::var("PG_CONFIG"))
        .unwrap_or_else(|_| "pg_config".to_string());

    let output = Command::new(&pg_config)
        .arg("--includedir-server")
        .output()
        .unwrap_or_else(|error| panic!("cannot run {pg_config}: {error}"));
    assert!(
        output.status.success(),
        "{pg_config} --includedir-server failed"
    );
    let include_dir = String::from_utf8(output.stdout).expect("pg_config output is UTF-8");

    cc::Build::new()
        .file("src/tls_guard.c")
        .include(include_dir.trim())
        .warnings(true)
        .compile("pgxila_tls_guard");
}
