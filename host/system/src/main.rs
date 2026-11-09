#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod platform;
#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
mod protocol;

fn main() {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    if platform::run().is_err() {
        eprintln!("system web engine failed; check runtime availability and profile access");
        std::process::exit(1);
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        eprintln!("system web engines are supported on Windows, Linux, and macOS 14+ only");
        std::process::exit(1);
    }
}
