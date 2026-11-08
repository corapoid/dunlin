//! OpenSSH askpass helper. OpenSSH reads the passphrase from this process's
//! standard output; this channel is the protocol, not an application log.

/// Write the askpass response directly to fd 1. This is the OpenSSH protocol,
/// not logging; keeping it out of formatting macros makes that boundary clear.
fn write_protocol(bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let fd = std::io::stdout().as_raw_fd();
        let mut offset = 0;
        while offset < bytes.len() {
            // SAFETY: `bytes` is valid for the remaining length and fd 1 is the
            // process's standard output.
            let written =
                unsafe { libc::write(fd, bytes[offset..].as_ptr().cast(), bytes.len() - offset) };
            if written < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if written == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "standard output closed",
                ));
            }
            offset += written as usize;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(bytes).and_then(|()| stdout.flush())
    }
}

fn main() {
    let Ok(item) = std::env::var("DUNLIN_SSH_KEYCHAIN_ITEM") else {
        eprintln!("dunlin-ssh-askpass: DUNLIN_SSH_KEYCHAIN_ITEM is not set");
        std::process::exit(1);
    };
    match dunlin_ssh::keychain::get_passphrase(&item) {
        Ok(passphrase) => {
            let mut response = passphrase.into_bytes();
            response.push(b'\n');
            if let Err(error) = write_protocol(&response) {
                eprintln!("dunlin-ssh-askpass: could not write the passphrase: {error}");
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("dunlin-ssh-askpass: {error}");
            std::process::exit(1);
        }
    }
}
