fn main() {
    println!("cargo::rustc-check-cfg=cfg(dotnet_value_multithreading)");
    if std::env::var_os("CARGO_FEATURE_MULTITHREADING").is_some() {
        println!("cargo::rustc-cfg=dotnet_value_multithreading");
    }
}
