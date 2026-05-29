use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUSTFLAGS_SEPARATOR: &str = "\x1f";

type Result<T> = std::result::Result<T, String>;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Arch {
    Base,
    Avx2,
    Pext,
    Arm64,
}

#[derive(Debug)]
struct Config {
    arch: Arch,
    target: String,
    pgo: bool,
    bench_depth: u16,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let config = parse_args()?;
    ensure_arch_target_pair(config.arch, &config.target)?;
    ensure_rust_target(&config.target)?;

    if config.pgo {
        build_with_pgo(&config)
    } else {
        let target_dir = target_dir("release", config.arch, &config.target);
        cargo_build(&config.target, config.arch, &target_dir, &[])?;
        copy_dist_binary(
            &binary_path(&target_dir, &config.target),
            config.arch,
            &config.target,
            false,
        )
    }
}

fn parse_args() -> Result<Config> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "build".to_string());
    if command == "help" || command == "--help" || command == "-h" {
        print_usage();
        std::process::exit(0);
    }
    if command != "build" {
        return Err(format!(
            "unknown command `{command}`; expected `build`. Run `cargo xtask help`."
        ));
    }

    let mut arch: Option<Arch> = None;
    let mut target: Option<String> = None;
    let mut pgo = false;
    let mut bench_depth = 13u16;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--arch" | "-a" => {
                let value = args
                    .next()
                    .ok_or_else(|| "`--arch` requires a value".to_string())?;
                arch = Some(parse_arch(&value)?);
            }
            "--target" | "-t" => {
                target = Some(
                    args.next()
                        .ok_or_else(|| "`--target` requires a value".to_string())?,
                );
            }
            "--pgo" => pgo = true,
            "--bench-depth" => {
                let value = args
                    .next()
                    .ok_or_else(|| "`--bench-depth` requires a value".to_string())?;
                bench_depth = value
                    .parse::<u16>()
                    .map_err(|_| format!("invalid bench depth `{value}`"))?;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }

    let arch = arch.unwrap_or_else(default_arch);
    let target = target.unwrap_or_else(|| default_target(arch));
    Ok(Config {
        arch,
        target,
        pgo,
        bench_depth,
    })
}

fn print_usage() {
    println!(
        "Usage:\n  cargo xtask build [--arch base|x86-64|avx2|pext|arm64] [--target <triple>] [--pgo] [--bench-depth <n>]\n\nExamples:\n  cargo xtask build\n  cargo xtask build --arch avx2\n  cargo xtask build --arch pext --pgo\n  cargo xtask build --arch arm64 --target aarch64-apple-darwin"
    );
}

fn parse_arch(value: &str) -> Result<Arch> {
    match value.to_ascii_lowercase().as_str() {
        "base" | "x86-64" | "x86_64" | "x64" => Ok(Arch::Base),
        "avx2" => Ok(Arch::Avx2),
        "pext" | "bmi2" => Ok(Arch::Pext),
        "arm64" | "aarch64" => Ok(Arch::Arm64),
        _ => Err(format!(
            "unknown arch `{value}`; expected base, avx2, pext, or arm64"
        )),
    }
}

fn default_arch() -> Arch {
    let host = host_triple().unwrap_or_default();
    if host.starts_with("aarch64-") {
        Arch::Arm64
    } else {
        Arch::Base
    }
}

fn default_target(arch: Arch) -> String {
    let host = host_triple().unwrap_or_default();
    let os = if host.contains("windows") {
        "windows"
    } else if host.contains("apple-darwin") {
        "macos"
    } else {
        "linux"
    };

    match (arch, os) {
        (Arch::Arm64, "windows") => "aarch64-pc-windows-msvc",
        (Arch::Arm64, "macos") => "aarch64-apple-darwin",
        (Arch::Arm64, _) => "aarch64-unknown-linux-gnu",
        (_, "windows") => "x86_64-pc-windows-msvc",
        (_, "macos") => "x86_64-apple-darwin",
        (_, _) => "x86_64-unknown-linux-gnu",
    }
    .to_string()
}

fn ensure_arch_target_pair(arch: Arch, target: &str) -> Result<()> {
    match arch {
        Arch::Base | Arch::Avx2 | Arch::Pext if !target.starts_with("x86_64-") => Err(format!(
            "`--arch {}` requires an x86_64 target, got `{target}`",
            arch_arg_name(arch)
        )),
        Arch::Arm64 if !target.starts_with("aarch64-") => Err(format!(
            "`--arch arm64` requires an aarch64 target, got `{target}`"
        )),
        _ => Ok(()),
    }
}

fn arch_arg_name(arch: Arch) -> &'static str {
    match arch {
        Arch::Base => "base",
        Arch::Avx2 => "avx2",
        Arch::Pext => "pext",
        Arch::Arm64 => "arm64",
    }
}

fn asset_arch_name(arch: Arch) -> &'static str {
    match arch {
        Arch::Base => "x86-64",
        Arch::Avx2 => "avx2",
        Arch::Pext => "pext",
        Arch::Arm64 => "arm64",
    }
}

fn rustflags(arch: Arch) -> Vec<String> {
    match arch {
        Arch::Base => vec!["-C".into(), "target-cpu=x86-64".into()],
        Arch::Avx2 => vec!["-C".into(), "target-cpu=x86-64-v3".into()],
        Arch::Pext => vec![
            "--cfg".into(),
            "lynx_pext".into(),
            "-C".into(),
            "target-cpu=x86-64-v3".into(),
            "-C".into(),
            "target-feature=+bmi2".into(),
        ],
        Arch::Arm64 => vec!["-C".into(), "target-cpu=generic".into()],
    }
}

fn build_with_pgo(config: &Config) -> Result<()> {
    let host = host_triple()?;
    if config.target != host {
        return Err(format!(
            "PGO training must run the instrumented binary locally, so target `{}` must match host `{host}`",
            config.target
        ));
    }

    let llvm_profdata = ensure_llvm_profdata()?;
    let pgo_dir = PathBuf::from("target")
        .join("pgo")
        .join(sanitize(&config.target))
        .join(arch_arg_name(config.arch));
    let raw_dir = pgo_dir.join("raw");
    if raw_dir.exists() {
        fs::remove_dir_all(&raw_dir)
            .map_err(|err| format!("failed to remove `{}`: {err}", raw_dir.display()))?;
    }
    fs::create_dir_all(&raw_dir)
        .map_err(|err| format!("failed to create `{}`: {err}", raw_dir.display()))?;

    let mut generate_flags = vec![
        "-C".to_string(),
        format!("profile-generate={}", raw_dir.display()),
    ];
    generate_flags.extend(rustflags(config.arch));

    let gen_target_dir = target_dir("pgo-gen", config.arch, &config.target);
    cargo_build(
        &config.target,
        config.arch,
        &gen_target_dir,
        &generate_flags,
    )?;

    let instrumented = binary_path(&gen_target_dir, &config.target);
    run_training_bench(&instrumented, &raw_dir, config.bench_depth)?;

    let profdata = pgo_dir.join("lynx.profdata");
    merge_profiles(&llvm_profdata, &raw_dir, &profdata)?;

    let mut use_flags = vec![
        "-C".to_string(),
        format!("profile-use={}", profdata.display()),
    ];
    use_flags.extend(rustflags(config.arch));

    let use_target_dir = target_dir("pgo-use", config.arch, &config.target);
    cargo_build(&config.target, config.arch, &use_target_dir, &use_flags)?;
    copy_dist_binary(
        &binary_path(&use_target_dir, &config.target),
        config.arch,
        &config.target,
        true,
    )
}

fn cargo_build(
    target: &str,
    arch: Arch,
    target_dir: &Path,
    override_flags: &[String],
) -> Result<()> {
    let flags = if override_flags.is_empty() {
        rustflags(arch)
    } else {
        override_flags.to_vec()
    };

    println_flush(format_args!(
        "Building Lynx {} for {}{}",
        asset_arch_name(arch),
        target,
        if override_flags.is_empty() {
            ""
        } else {
            " with PGO flags"
        }
    ));

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg(target)
        .arg("--target-dir")
        .arg(target_dir)
        .env("CARGO_ENCODED_RUSTFLAGS", flags.join(RUSTFLAGS_SEPARATOR))
        .env_remove("RUSTFLAGS")
        .status()
        .map_err(|err| format!("failed to run cargo build: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo build failed with status {status}"))
    }
}

fn run_training_bench(binary: &Path, raw_dir: &Path, depth: u16) -> Result<()> {
    println_flush(format_args!(
        "Training PGO profile with internal bench depth {depth}"
    ));
    let profile_pattern = raw_dir.join("lynx-%p-%m.profraw");
    let mut child = Command::new(binary)
        .env("LLVM_PROFILE_FILE", &profile_pattern)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| format!("failed to run `{}`: {err}", binary.display()))?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "failed to open engine stdin".to_string())?;
        writeln!(stdin, "bench {depth}").map_err(|err| format!("failed to start bench: {err}"))?;
        stdin
            .flush()
            .map_err(|err| format!("failed to flush stdin: {err}"))?;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to open engine stdout".to_string())?;
    let reader = BufReader::new(stdout);
    let mut saw_summary = false;
    for line in reader.lines() {
        let line = line.map_err(|err| format!("failed reading engine output: {err}"))?;
        println_flush(format_args!("{line}"));
        if line.starts_with("Nodes/second") {
            saw_summary = true;
            break;
        }
    }

    if let Some(stdin) = child.stdin.as_mut() {
        let _ = writeln!(stdin, "quit");
    }

    let status = child
        .wait()
        .map_err(|err| format!("failed waiting for engine: {err}"))?;
    if !status.success() {
        return Err(format!("training bench exited with status {status}"));
    }
    if !saw_summary {
        return Err("training bench did not produce a bench summary".to_string());
    }
    Ok(())
}

fn merge_profiles(llvm_profdata: &Path, raw_dir: &Path, profdata: &Path) -> Result<()> {
    let mut inputs = Vec::new();
    for entry in fs::read_dir(raw_dir)
        .map_err(|err| format!("failed to read `{}`: {err}", raw_dir.display()))?
    {
        let path = entry
            .map_err(|err| format!("failed to read profile entry: {err}"))?
            .path();
        if path.extension() == Some(OsStr::new("profraw")) {
            inputs.push(path);
        }
    }
    if inputs.is_empty() {
        return Err(format!(
            "no .profraw files found in `{}`",
            raw_dir.display()
        ));
    }

    println_flush(format_args!("Merging {} profile file(s)", inputs.len()));
    let status = Command::new(llvm_profdata)
        .arg("merge")
        .arg("-output")
        .arg(profdata)
        .args(&inputs)
        .status()
        .map_err(|err| format!("failed to run `{}`: {err}", llvm_profdata.display()))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("llvm-profdata merge failed with status {status}"))
    }
}

fn ensure_rust_target(target: &str) -> Result<()> {
    if find_on_path("rustup").is_none() {
        eprintln!(
            "rustup not found; if `{target}` is not installed, run `rustup target add {target}`."
        );
        return Ok(());
    }

    let status = Command::new("rustup")
        .arg("target")
        .arg("add")
        .arg(target)
        .status()
        .map_err(|err| format!("failed to run rustup target add: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to install target `{target}`; run `rustup target add {target}` manually"
        ))
    }
}

fn ensure_llvm_profdata() -> Result<PathBuf> {
    if let Some(path) = find_llvm_profdata() {
        return Ok(path);
    }

    if find_on_path("rustup").is_none() {
        return Err(
            "llvm-profdata was not found. Install it with `rustup component add llvm-tools-preview` or add LLVM's bin directory to PATH."
                .to_string(),
        );
    }

    println_flush(format_args!(
        "Installing llvm-tools-preview for PGO support"
    ));
    let status = Command::new("rustup")
        .arg("component")
        .arg("add")
        .arg("llvm-tools-preview")
        .status()
        .map_err(|err| format!("failed to run rustup component add: {err}"))?;
    if !status.success() {
        return Err(
            "failed to install llvm-tools-preview; run `rustup component add llvm-tools-preview` manually"
                .to_string(),
        );
    }

    find_llvm_profdata().ok_or_else(|| {
        "llvm-profdata was still not found after installing llvm-tools-preview".to_string()
    })
}

fn find_llvm_profdata() -> Option<PathBuf> {
    find_on_path("llvm-profdata").or_else(find_rustup_llvm_profdata)
}

fn find_rustup_llvm_profdata() -> Option<PathBuf> {
    let sysroot = command_output("rustc", &["--print", "sysroot"]).ok()?;
    let host = host_triple().ok()?;
    let candidate = PathBuf::from(sysroot)
        .join("lib")
        .join("rustlib")
        .join(host)
        .join("bin")
        .join(tool_name("llvm-profdata"));
    candidate.exists().then_some(candidate)
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    for dir in env::split_paths(&paths) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        if cfg!(windows) && !program.ends_with(".exe") {
            let candidate = dir.join(format!("{program}.exe"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn tool_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn host_triple() -> Result<String> {
    let output = command_output("rustc", &["-vV"])?;
    output
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .ok_or_else(|| "failed to parse rustc host triple".to_string())
}

fn command_output(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("failed to run `{program}`: {err}"))?;
    if !output.status.success() {
        return Err(format!("`{program}` failed with status {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn target_dir(kind: &str, arch: Arch, target: &str) -> PathBuf {
    PathBuf::from("target").join("xtask").join(format!(
        "{kind}-{}-{}",
        sanitize(target),
        arch_arg_name(arch)
    ))
}

fn binary_path(target_dir: &Path, target: &str) -> PathBuf {
    target_dir
        .join(target)
        .join("release")
        .join(format!("lynx{}", exe_suffix(target)))
}

fn copy_dist_binary(binary: &Path, arch: Arch, target: &str, pgo: bool) -> Result<()> {
    if !binary.exists() {
        return Err(format!(
            "expected binary `{}` does not exist",
            binary.display()
        ));
    }
    let dist = PathBuf::from("target").join("dist");
    fs::create_dir_all(&dist)
        .map_err(|err| format!("failed to create `{}`: {err}", dist.display()))?;
    let asset = dist.join(asset_name(arch, target, pgo)?);
    fs::copy(binary, &asset).map_err(|err| {
        format!(
            "failed to copy `{}` to `{}`: {err}",
            binary.display(),
            asset.display()
        )
    })?;
    println_flush(format_args!("Built {}", asset.display()));
    Ok(())
}

fn println_flush(args: std::fmt::Arguments<'_>) {
    println!("{args}");
    io::stdout().flush().expect("stdout flush failed");
}

fn asset_name(arch: Arch, target: &str, pgo: bool) -> Result<String> {
    let pgo_suffix = if pgo { "-pgo" } else { "" };
    Ok(format!(
        "lynx-v{}-{}-{}{}{}",
        package_version()?,
        os_name(target),
        asset_arch_name(arch),
        pgo_suffix,
        exe_suffix(target)
    ))
}

fn package_version() -> Result<String> {
    let manifest = fs::read_to_string("Cargo.toml")
        .map_err(|err| format!("failed to read Cargo.toml: {err}"))?;
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(version) = line.strip_prefix("version = ") {
            return Ok(version.trim_matches('"').to_string());
        }
    }
    Err("failed to find package version in Cargo.toml".to_string())
}

fn os_name(target: &str) -> &'static str {
    if target.contains("windows") {
        "windows"
    } else if target.contains("apple-darwin") {
        "macos"
    } else {
        "linux"
    }
}

fn exe_suffix(target: &str) -> &'static str {
    if target.contains("windows") {
        ".exe"
    } else {
        ""
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}
