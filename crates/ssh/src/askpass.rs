//! OpenSSH askpass helper. OpenSSH reads the passphrase from this process's
//! standard output; this channel is the protocol, not an application log.

use std::io::Write;

fn main() {
    let Ok(item) = std::env::var("DUNLIN_SSH_KEYCHAIN_ITEM") else {
        eprintln!("dunlin-ssh-askpass: DUNLIN_SSH_KEYCHAIN_ITEM is not set");
        std::process::exit(1);
    };
    match dunlin_ssh::keychain::get_passphrase(&item) {
        Ok(passphrase) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(error) = stdout
                .write_all(passphrase.as_bytes())
                .and_then(|()| stdout.write_all(b"\n"))
                .and_then(|()| stdout.flush())
            {
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
