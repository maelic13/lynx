fn main() {
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    let mut build = cc::Build::new();
    build
        .file("vendor/fathom/src/tbprobe.c")
        .include("vendor/fathom/src")
        .define("TB_NO_HELPER_API", None)
        .warnings(false);

    if target_env == "msvc" {
        build
            .cpp(true)
            .flag_if_supported("/TP")
            .flag_if_supported("/std:c++17");
    } else {
        build.flag_if_supported("-std=c11");
    }

    build.compile("fathom");

    if std::env::var("CARGO_CFG_UNIX").is_ok() {
        println!("cargo:rustc-link-lib=pthread");
    }
}
