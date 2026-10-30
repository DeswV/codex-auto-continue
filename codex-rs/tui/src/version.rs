/// The current Codex CLI version as embedded at compile time.
pub const CODEX_CLI_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+auto_continue.0.1.0");
