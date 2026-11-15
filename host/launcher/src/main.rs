fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(
        spotcore_engine::run_embedded().unwrap_or_else(spotcore_engine::run_standalone),
    )
}
