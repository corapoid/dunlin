use dunlin_domain::SshHost;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedHost {
    pub hostname: String,
    pub user: String,
    pub port: u16,
    pub identity: Option<String>,
    pub proxy_jump: Option<String>,
}

pub struct BootstrapResult {
    pub pairing_code: String,
    pub tunnel: Tunnel,
}

pub struct Tunnel {
    child: Child,
    pub local_port: u16,
    pub relay_port: u16,
}

impl Tunnel {
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn resolve(target: &str) -> Result<ResolvedHost, String> {
    validate_target(target)?;
    let output = Command::new(ssh_program())
        .args(["-G", "--", target])
        .output()
        .map_err(|error| format!("Could not run OpenSSH: {error}"))?;
    if !output.status.success() {
        return Err(command_error("ssh -G", &output.stderr));
    }
    Ok(parse_ssh_config(
        &String::from_utf8_lossy(&output.stdout),
        target,
    ))
}

fn parse_ssh_config(text: &str, target: &str) -> ResolvedHost {
    let mut hostname = target.to_string();
    let mut user = String::new();
    let mut port = 22u16;
    let mut identity = None;
    let mut proxy_jump = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key {
            "hostname" => hostname = value.to_string(),
            "user" => user = value.to_string(),
            "port" => port = value.parse().unwrap_or(22),
            "identityfile" => identity = Some(value.to_string()),
            "proxyjump" => {
                proxy_jump = (value != "none" && !value.is_empty()).then(|| value.to_string())
            }
            _ => {}
        }
    }
    ResolvedHost {
        hostname,
        user,
        port,
        identity,
        proxy_jump,
    }
}

pub fn import_aliases() -> Result<Vec<String>, String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(Vec::new());
    };
    let path = PathBuf::from(home).join(".ssh/config");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut aliases = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("Host ") else {
            continue;
        };
        for alias in rest.split_whitespace() {
            if alias.starts_with('!')
                || alias.contains('*')
                || alias.contains('?')
                || alias.len() > 120
            {
                continue;
            }
            if !aliases.iter().any(|existing| existing == alias) {
                aliases.push(alias.to_string());
            }
        }
    }
    aliases.sort();
    Ok(aliases)
}

pub fn control_path(host: &SshHost) -> Result<PathBuf, String> {
    let directory = control_dir()?;
    let digest = hex::encode(Sha256::digest(host.id.as_bytes()));
    Ok(directory.join(format!("dunlin-{}.sock", &digest[..16])))
}

fn control_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
    let home = PathBuf::from(home);
    let directory = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Dunlin/ssh")
    } else {
        home.join(".local/share/dunlin/ssh")
    };
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create the SSH control directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700));
    }
    Ok(directory)
}

pub fn ensure_master(host: &SshHost, askpass: Option<&Path>) -> Result<(), String> {
    if check_master(host) {
        return Ok(());
    }
    let control = control_path(host)?;
    let mut command = Command::new(ssh_program());
    command.args(master_args(host, &control, askpass.is_none()));
    command.args(["--", &host.target, "true"]);
    configure_askpass(&mut command, host, askpass);
    let output = command
        .output()
        .map_err(|error| format!("Could not start OpenSSH: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "SSH master connection failed: {}. Open an SSH terminal first to verify the host key.",
            command_error("ssh", &output.stderr)
        ))
    }
}

pub fn check_master(host: &SshHost) -> bool {
    let Ok(control) = control_path(host) else {
        return false;
    };
    Command::new(ssh_program())
        .args([
            "-O",
            "check",
            "-o",
            &format!("ControlPath={}", control.display()),
            "--",
            &host.target,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn exit_master(host: &SshHost) {
    if let Ok(control) = control_path(host) {
        let _ = Command::new(ssh_program())
            .args([
                "-O",
                "exit",
                "-o",
                &format!("ControlPath={}", control.display()),
                "--",
                &host.target,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

pub fn bootstrap(
    host: &SshHost,
    relay_binary: &Path,
    askpass: Option<&Path>,
) -> Result<BootstrapResult, String> {
    if !relay_binary.is_file() {
        return Err("The dunlin-relay binary is unavailable".into());
    }
    ensure_master(host, askpass)?;
    let control = control_path(host)?;
    let remote_binary = "$HOME/.dunlin/bin/dunlin-relay";
    let mkdir = run_remote(
        host,
        &control,
        "mkdir -p \"$HOME/.dunlin/bin\" && chmod 700 \"$HOME/.dunlin\"",
        askpass,
    )?;
    if !mkdir.status.success() {
        return Err(format!(
            "Could not prepare the remote relay directory: {}",
            command_error("ssh", &mkdir.stderr)
        ));
    }
    let upload = Command::new(scp_program())
        .args([
            "-q",
            "-o",
            &format!("ControlPath={}", control.display()),
            "--",
            &relay_binary.to_string_lossy(),
            &format!("{}:{}", host.target, remote_binary),
        ])
        .output()
        .map_err(|error| format!("Could not start scp: {error}"))?;
    if !upload.status.success() {
        return Err(format!(
            "Could not upload dunlin-relay: {}",
            command_error("scp", &upload.stderr)
        ));
    }
    let local_hash = hex::encode(Sha256::digest(
        std::fs::read(relay_binary).map_err(|e| e.to_string())?,
    ));
    let script = bootstrap_script(host, &local_hash);
    let result = run_remote_script(host, &control, &script, askpass)?;
    if !result.status.success() {
        return Err(format!(
            "Remote relay bootstrap failed: {}",
            command_error("ssh", &result.stderr)
        ));
    }
    let pairing_code = pair(host, &host.user, askpass)?;
    let tunnel = start_tunnel(host, askpass)?;
    Ok(BootstrapResult {
        pairing_code,
        tunnel,
    })
}

pub fn pair(host: &SshHost, user: &str, askpass: Option<&Path>) -> Result<String, String> {
    let control = control_path(host)?;
    let command = format!(
        "\"$HOME/.dunlin/bin/dunlin-relay\" pair {} --ttl 15",
        shell_quote(user)
    );
    let output = run_remote(host, &control, &command, askpass)?;
    if !output.status.success() {
        return Err(format!(
            "Could not create a relay pairing code: {}",
            command_error("ssh", &output.stderr)
        ));
    }
    parse_pairing_code(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "The relay did not print a pairing code".into())
}

pub fn start_tunnel(host: &SshHost, askpass: Option<&Path>) -> Result<Tunnel, String> {
    ensure_master(host, askpass)?;
    let control = control_path(host)?;
    let local_port = if host.local_port != 0 {
        host.local_port
    } else {
        free_loopback_port()?
    };
    let relay_port = if host.relay_port == 0 {
        8787
    } else {
        host.relay_port
    };
    let mut command = Command::new(ssh_program());
    command.args([
        "-N",
        "-o",
        &format!("ControlPath={}", control.display()),
        "-o",
        "ExitOnForwardFailure=yes",
        "-L",
        &format!("127.0.0.1:{local_port}:127.0.0.1:{relay_port}"),
        "--",
        &host.target,
    ]);
    configure_askpass(&mut command, host, askpass);
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start the SSH tunnel: {error}"))?;
    let mut tunnel = Tunnel {
        child,
        local_port,
        relay_port,
    };
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(
            &format!("127.0.0.1:{local_port}")
                .parse()
                .map_err(|_| "Invalid tunnel port")?,
            Duration::from_millis(200),
        )
        .is_ok()
        {
            return Ok(tunnel);
        }
        if let Ok(Some(status)) = tunnel.child.try_wait() {
            return Err(format!("SSH tunnel exited with {status}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    tunnel.stop();
    Err("SSH tunnel did not become reachable on loopback".into())
}

fn master_args(host: &SshHost, control: &Path, batch: bool) -> Vec<String> {
    let mut args = vec![
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}", control.display()),
        "-o".into(),
        "ControlPersist=300".into(),
        "-o".into(),
        "ConnectTimeout=15".into(),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        if batch {
            "BatchMode=yes".into()
        } else {
            "BatchMode=no".into()
        },
    ];
    if host.port != 0 && host.port != 22 {
        args.push("-p".into());
        args.push(host.port.to_string());
    }
    if let Some(identity) = host
        .identity
        .as_deref()
        .filter(|path| !path.trim().is_empty())
    {
        args.push("-i".into());
        args.push(identity.to_string());
    }
    args
}

fn run_remote(
    host: &SshHost,
    control: &Path,
    command: &str,
    askpass: Option<&Path>,
) -> Result<std::process::Output, String> {
    let mut process = Command::new(ssh_program());
    process.args([
        "-o",
        &format!("ControlPath={}", control.display()),
        "--",
        &host.target,
        command,
    ]);
    configure_askpass(&mut process, host, askpass);
    process
        .output()
        .map_err(|error| format!("Could not run the remote command: {error}"))
}

fn run_remote_script(
    host: &SshHost,
    control: &Path,
    script: &str,
    askpass: Option<&Path>,
) -> Result<std::process::Output, String> {
    let mut process = Command::new(ssh_program());
    process.args([
        "-o",
        &format!("ControlPath={}", control.display()),
        "--",
        &host.target,
        "sh -s",
    ]);
    configure_askpass(&mut process, host, askpass);
    process.stdin(Stdio::piped());
    process.stdout(Stdio::piped());
    process.stderr(Stdio::piped());
    let mut child = process
        .spawn()
        .map_err(|error| format!("Could not start the remote bootstrap: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .map_err(|error| format!("Could not send the bootstrap script: {error}"))?;
    }
    child
        .wait_with_output()
        .map_err(|error| format!("Could not read the bootstrap result: {error}"))
}

fn bootstrap_script(host: &SshHost, expected_hash: &str) -> String {
    let port = if host.relay_port == 0 {
        8787
    } else {
        host.relay_port
    };
    format!(
        r#"set -eu
BIN="$HOME/.dunlin/bin/dunlin-relay"
chmod 700 "$BIN"
if command -v shasum >/dev/null 2>&1; then
  ACTUAL=$(shasum -a 256 "$BIN" | awk '{{print $1}}')
elif command -v sha256sum >/dev/null 2>&1; then
  ACTUAL=$(sha256sum "$BIN" | awk '{{print $1}}')
else
  echo "No SHA-256 tool on the host" >&2
  exit 1
fi
if [ "$ACTUAL" != "{expected_hash}" ]; then
  echo "dunlin-relay checksum mismatch" >&2
  exit 1
fi
if [ "$(uname -s)" = "Darwin" ]; then
  PLIST="$HOME/Library/LaunchAgents/dev.dunlin.relay.plist"
  mkdir -p "$HOME/Library/LaunchAgents"
  cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>dev.dunlin.relay</string>
<key>ProgramArguments</key><array><string>$BIN</string><string>serve</string><string>--listen</string><string>127.0.0.1:{port}</string></array>
<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
</dict></plist>
EOF
  launchctl bootout "gui/$(id -u)/dev.dunlin.relay" >/dev/null 2>&1 || true
  launchctl bootstrap "gui/$(id -u)" "$PLIST"
  launchctl kickstart -k "gui/$(id -u)/dev.dunlin.relay"
else
  mkdir -p "$HOME/.config/systemd/user"
  cat > "$HOME/.config/systemd/user/dunlin-relay.service" <<EOF
[Unit]
Description=Dunlin relay
After=network.target
[Service]
ExecStart=%h/.dunlin/bin/dunlin-relay serve --listen 127.0.0.1:{port}
Restart=on-failure
[Install]
WantedBy=default.target
EOF
  systemctl --user daemon-reload
  systemctl --user enable --now dunlin-relay.service
fi
"#
    )
}

fn configure_askpass(command: &mut Command, host: &SshHost, askpass: Option<&Path>) {
    if let Some(askpass) = askpass {
        command.env("SSH_ASKPASS", askpass);
        command.env("SSH_ASKPASS_REQUIRE", "force");
        command.env("DISPLAY", ":0");
        command.env("DUNLIN_SSH_KEYCHAIN_ITEM", &host.id);
    }
}

fn parse_pairing_code(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("Pairing code for ")
            .and_then(|rest| rest.split_once(": "))
            .map(|(_, code)| code.trim().to_string())
            .filter(|code| !code.is_empty())
    })
}

fn free_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("Could not reserve a loopback port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("Could not read the loopback port: {error}"))
}

fn validate_target(target: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty()
        || target.len() > 256
        || target.starts_with('-')
        || target.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'-' | b'@' | b':' | b'[' | b']' | b'%'))
        })
        || target.matches('@').count() > 1
    {
        return Err("Enter an SSH host or alias without spaces or options".into());
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn ssh_program() -> &'static str {
    if Path::new("/usr/bin/ssh").is_file() {
        "/usr/bin/ssh"
    } else {
        "ssh"
    }
}

fn scp_program() -> &'static str {
    if Path::new("/usr/bin/scp").is_file() {
        "/usr/bin/scp"
    } else {
        "scp"
    }
}

fn command_error(command: &str, stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.is_empty() {
        format!("{command} failed")
    } else {
        text.chars().take(400).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_openssh_config_output() {
        let parsed = parse_ssh_config(
            "hostname example.test\nuser alice\nport 2222\nidentityfile ~/.ssh/id_ed25519\nproxyjump none\n",
            "alias",
        );
        assert_eq!(parsed.hostname, "example.test");
        assert_eq!(parsed.user, "alice");
        assert_eq!(parsed.port, 2222);
        assert_eq!(parsed.identity.as_deref(), Some("~/.ssh/id_ed25519"));
        assert!(parsed.proxy_jump.is_none());
    }

    #[test]
    fn target_validation_rejects_options_and_spaces() {
        assert!(validate_target("user@host").is_ok());
        assert!(validate_target("-oProxyCommand=bad").is_err());
        assert!(validate_target("host name").is_err());
    }

    #[test]
    fn pairing_codes_are_parsed() {
        assert_eq!(
            parse_pairing_code("Pairing code for admin: ABC123\nExpires in 15 minutes"),
            Some("ABC123".into())
        );
        assert_eq!(parse_pairing_code("no code"), None);
    }

    #[test]
    fn bootstrap_script_contains_service_installers() {
        let host = SshHost {
            id: "host".into(),
            name: "Host".into(),
            target: "host".into(),
            user: "user".into(),
            port: 22,
            identity: None,
            relay_port: 8787,
            local_port: 0,
            host_key: String::new(),
            created_at: 0,
        };
        let script = bootstrap_script(&host, "abc");
        assert!(script.contains("dev.dunlin.relay.plist"));
        assert!(script.contains("systemctl --user enable --now dunlin-relay.service"));
        assert!(script.contains("127.0.0.1:8787"));
        assert!(script.contains("checksum mismatch"));
    }

    #[test]
    fn config_parser_preserves_defaults_and_trims_openssh_values() {
        let empty = parse_ssh_config("unknown value\nmalformed\n", "saved-alias");
        assert_eq!(empty.hostname, "saved-alias");
        assert_eq!(empty.user, "");
        assert_eq!(empty.port, 22);
        assert_eq!(empty.identity, None);
        assert_eq!(empty.proxy_jump, None);

        let parsed = parse_ssh_config(
            "hostname   actual.test  \nuser   bob \nport invalid\nidentityfile   /keys/my key \nproxyjump   gateway \n",
            "alias",
        );
        assert_eq!(parsed.hostname, "actual.test");
        assert_eq!(parsed.user, "bob");
        assert_eq!(parsed.port, 22);
        assert_eq!(parsed.identity.as_deref(), Some("/keys/my key"));
        assert_eq!(parsed.proxy_jump.as_deref(), Some("gateway"));
        for value in ["", "none", "   ", " none "] {
            assert_eq!(
                parse_ssh_config(&format!("proxyjump {value}\n"), "alias").proxy_jump,
                None
            );
        }
    }
}
