//! Prints the version of the libhdf5 this build is linked against, and can check it (IP-01 phase 1I; CI runs it on every test leg).
//!
//! ```text
//! cargo run -p build-log-io --example libhdf5_version -- [--at-least X.Y.Z] [--exactly X.Y.Z]
//! ```
//!
//! The version is always the only thing on standard output, as `major.minor.micro`. A check that fails is explained on standard error and the
//! exit status is 1; a command line it does not understand exits with 2.

use std::process::ExitCode;

use build_log_io::{libhdf5_version, Libhdf5Version};

fn main() -> ExitCode {
    let mut at_least: Option<Libhdf5Version> = None;
    let mut exactly: Option<Libhdf5Version> = None;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let target = match flag.as_str() {
            "--at-least" => &mut at_least,
            "--exactly" => &mut exactly,
            _ => return usage(&format!("unknown argument {flag:?}")),
        };
        let Some(text) = args.next() else {
            return usage(&format!("{flag} needs a version"));
        };
        match text.parse() {
            Ok(version) => *target = Some(version),
            Err(error) => return usage(&format!("{flag}: {error}")),
        }
    }

    let found = libhdf5_version();
    println!("{found}");

    let mut passed = true;
    if let Some(minimum) = at_least {
        if found < minimum {
            eprintln!("libhdf5 {found} is older than the minimum {minimum}");
            passed = false;
        }
    }
    if let Some(expected) = exactly {
        if found != expected {
            eprintln!("libhdf5 {found} was expected to be exactly {expected}");
            passed = false;
        }
    }
    if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn usage(problem: &str) -> ExitCode {
    eprintln!("{problem}");
    eprintln!("usage: libhdf5_version [--at-least X.Y.Z] [--exactly X.Y.Z]");
    ExitCode::from(2)
}
