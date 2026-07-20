use std::collections::HashMap;

/// SSH connection settings for a profile of type 'ssh'.
/// Mirrors `SshProfile` in the Hyper config (`typings/config.d.ts`).
#[derive(Debug, Clone, Default)]
pub struct SshOptions {
    pub host: String,
    pub user: String,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub forward_agent: bool,
    pub extra_args: Vec<String>,
    pub password_auth: bool,
    pub password: Option<String>,
}

/// Argv passed to the `ssh` binary. Port of `app/ui/ssh-args.ts` —
/// options first, then `user@host`, deterministic order.
pub fn build_ssh_args(ssh: &SshOptions) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(port) = ssh.port {
        args.push("-p".into());
        args.push(port.to_string());
    }
    // `readyTimeout: 15000` in the ssh2 client: fail in 15s rather than
    // hanging on the OS TCP timeout when a host is unreachable.
    args.push("-o".into());
    args.push("ConnectTimeout=15".into());
    if ssh.password_auth {
        // The ssh2 client authenticated with `tryKeyboard: true` and answered
        // keyboard-interactive prompts with the same password, which is the
        // only method many PAM setups offer — excluding it here would fail
        // connections the Electron build made fine.
        args.push("-o".into());
        args.push("PreferredAuthentications=password,keyboard-interactive".into());
        args.push("-o".into());
        args.push("PubkeyAuthentication=no".into());
        // ssh2 had no hostVerifier, so it accepted unknown host keys without
        // prompting. An interactive yes/no prompt cannot be answered by the
        // askpass helper, so keep the no-prompt behavior — but only for *new*
        // hosts: `accept-new` still refuses a key that changed under us.
        args.push("-o".into());
        args.push("StrictHostKeyChecking=accept-new".into());
    } else if let Some(identity) = &ssh.identity_file {
        if !identity.is_empty() {
            args.push("-i".into());
            args.push(identity.clone());
        }
    }
    if ssh.forward_agent {
        args.push("-A".into());
    }
    args.extend(ssh.extra_args.iter().cloned());
    args.push(format!("{}@{}", ssh.user, ssh.host));
    args
}

/// Shell command + env for launching an ssh session in a PTY.
///
/// Password auth spawns the system `ssh` with SSH_ASKPASS pointed at our own
/// binary in askpass-helper mode; the password is handed over via a 0600
/// tempfile named in HYPER_ASKPASS_FILE. This replaces the embedded JS ssh2
/// client of the Electron app.
///
/// `password_file` must be deleted once the session ends — the file has to
/// outlive individual prompts (ssh retries, and keyboard-interactive can ask
/// more than once), so the helper itself cannot remove it.
pub struct SshCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub password_file: Option<std::path::PathBuf>,
}

/// Write a password where the askpass helper can read it. `tempfile` creates
/// with 0600 on unix; the explicit set_permissions before the write keeps that
/// guarantee if the crate's default ever loosens.
pub fn write_password_file(password: &str) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    let mut file = tempfile::Builder::new()
        .prefix("hyper-askpass-")
        .tempfile()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(password.as_bytes())?;
    file.flush()?;
    let (_, path) = file.keep().map_err(|e| e.error)?;
    Ok(path)
}

pub fn build_ssh_command(
    ssh: &SshOptions,
    askpass_helper: Option<&std::path::Path>,
) -> std::io::Result<SshCommand> {
    let args = build_ssh_args(ssh);
    let mut env = HashMap::new();
    let mut password_file = None;

    if ssh.password_auth {
        match (&ssh.password, askpass_helper) {
            (Some(password), Some(helper)) if !password.is_empty() => {
                let path = write_password_file(password)?;
                env.insert("SSH_ASKPASS".into(), helper.display().to_string());
                // ssh runs inside our PTY and therefore *has* a controlling
                // terminal; without REQUIRE=force it would ignore SSH_ASKPASS
                // and prompt inline, defeating the saved password.
                env.insert("SSH_ASKPASS_REQUIRE".into(), "force".into());
                env.insert("HYPER_ASKPASS_FILE".into(), path.display().to_string());
                password_file = Some(path);
            }
            _ => {
                // No stored password: let ssh prompt on the TTY as usual.
            }
        }
    }

    Ok(SshCommand {
        program: "ssh".into(),
        args,
        env,
        password_file,
    })
}

/// Does this askpass prompt want the account password?
///
/// ssh passes the prompt text as argv[1] and uses the same askpass channel for
/// host-key confirmation ("Are you sure you want to continue connecting?") and
/// key passphrases. Printing the password at those prompts would both leak it
/// and answer the wrong question, so only password-ish prompts are answered.
fn prompt_wants_password(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    if lower.contains("passphrase") {
        return false;
    }
    // Host-key and other yes/no confirmations.
    if lower.contains("yes/no") || lower.contains("fingerprint") || lower.contains("continue connecting")
    {
        return false;
    }
    // An empty prompt means ssh gave us no context; the only prompt we ask
    // for is the password, so answer it.
    prompt.trim().is_empty() || lower.contains("password")
}

/// Askpass helper mode: called by ssh as `SSH_ASKPASS`. Prints the password
/// from HYPER_ASKPASS_FILE. Returns true if handled.
///
/// The file is deliberately *not* deleted here: ssh may prompt more than once
/// (retries, keyboard-interactive), and a one-shot file turned the second
/// prompt into a failure. The spawning session removes it on exit.
pub fn run_askpass_helper() -> bool {
    let Ok(path) = std::env::var("HYPER_ASKPASS_FILE") else {
        return false;
    };
    let prompt = std::env::args().nth(1).unwrap_or_default();
    if !prompt_wants_password(&prompt) {
        // Answer nothing; ssh treats an empty response as a declined prompt.
        return true;
    }
    let Ok(password) = std::fs::read_to_string(&path) else {
        return false;
    };
    print!("{password}");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_publickey_with_identity_and_port() {
        let ssh = SshOptions {
            host: "example.com".into(),
            user: "kate".into(),
            port: Some(2222),
            identity_file: Some("~/.ssh/id_ed25519".into()),
            forward_agent: true,
            extra_args: vec!["-v".into()],
            ..Default::default()
        };
        assert_eq!(
            build_ssh_args(&ssh),
            vec![
                "-p",
                "2222",
                "-o",
                "ConnectTimeout=15",
                "-i",
                "~/.ssh/id_ed25519",
                "-A",
                "-v",
                "kate@example.com"
            ]
        );
    }

    #[test]
    fn args_password_auth_forces_password_prefs() {
        let ssh = SshOptions {
            host: "h".into(),
            user: "u".into(),
            password_auth: true,
            ..Default::default()
        };
        assert_eq!(
            build_ssh_args(&ssh),
            vec![
                "-o",
                "ConnectTimeout=15",
                "-o",
                // keyboard-interactive is what PAM-only servers offer, and the
                // ssh2 client answered it with the same password.
                "PreferredAuthentications=password,keyboard-interactive",
                "-o",
                "PubkeyAuthentication=no",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "u@h"
            ]
        );
    }

    /// The password must only ever answer a password prompt — never a
    /// host-key confirmation or a key passphrase.
    #[test]
    fn askpass_only_answers_password_prompts() {
        assert!(prompt_wants_password("kate@example.com's password: "));
        assert!(prompt_wants_password("Password:"));
        assert!(prompt_wants_password(""));

        assert!(!prompt_wants_password(
            "The authenticity of host 'h (1.2.3.4)' can't be established.\n\
             ED25519 key fingerprint is SHA256:abc.\n\
             Are you sure you want to continue connecting (yes/no/[fingerprint])?"
        ));
        assert!(!prompt_wants_password(
            "Enter passphrase for key '/home/kate/.ssh/id_ed25519':"
        ));
    }

    #[test]
    fn password_file_is_reported_for_cleanup() {
        let ssh = SshOptions {
            host: "h".into(),
            user: "u".into(),
            password_auth: true,
            password: Some("hunter2".into()),
            ..Default::default()
        };
        let helper = std::path::PathBuf::from("/usr/local/bin/hyper-revamp");
        let cmd = build_ssh_command(&ssh, Some(&helper)).expect("builds");
        let path = cmd.password_file.clone().expect("password file recorded");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "hunter2",
            "helper can read the password"
        );
        assert_eq!(
            cmd.env.get("HYPER_ASKPASS_FILE"),
            Some(&path.display().to_string())
        );
        assert_eq!(cmd.env.get("SSH_ASKPASS_REQUIRE"), Some(&"force".to_string()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_file(&path).ok();
    }

    /// No stored password ⇒ no askpass wiring, so ssh prompts on the TTY.
    #[test]
    fn no_password_means_no_askpass() {
        let ssh = SshOptions {
            host: "h".into(),
            user: "u".into(),
            password_auth: true,
            password: None,
            ..Default::default()
        };
        let helper = std::path::PathBuf::from("/usr/local/bin/hyper-revamp");
        let cmd = build_ssh_command(&ssh, Some(&helper)).expect("builds");
        assert!(cmd.password_file.is_none());
        assert!(cmd.env.is_empty());
    }
}
