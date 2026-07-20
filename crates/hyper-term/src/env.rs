use std::collections::HashMap;

/// Child environment additions, mirroring Hyper's `app/session.ts` init:
/// TERM/COLORTERM/LANG/TERM_PROGRAM(+_VERSION), then profile env on top.
pub fn base_env(profile_env: &HashMap<String, String>) -> HashMap<String, String> {
    let mut env = HashMap::new();

    let locale = sys_locale::get_locale()
        .unwrap_or_else(|| "en_US".into())
        .replace('-', "_");
    env.insert("LANG".into(), format!("{locale}.UTF-8"));

    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    // `TERM_PROGRAM: productName` — the literal package name, which user
    // scripts and prompt integrations match on (`[ "$TERM_PROGRAM" = ... ]`).
    env.insert("TERM_PROGRAM".into(), "hyper-revamp".into());
    env.insert(
        "TERM_PROGRAM_VERSION".into(),
        env!("CARGO_PKG_VERSION").into(),
    );

    for (k, v) in profile_env {
        env.insert(k.clone(), v.clone());
    }
    env
}

/// System default shell: `$SHELL`, else a sensible per-OS fallback.
/// Mirrors the `default-shell` package the Electron app used: `/bin/sh` is
/// the POSIX guarantee on Linux, where `/bin/bash` may not exist at all
/// (minimal containers).
pub fn default_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.is_empty() {
            return shell;
        }
    }
    if cfg!(target_os = "macos") {
        "/bin/zsh".into()
    } else if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        "/bin/sh".into()
    }
}
