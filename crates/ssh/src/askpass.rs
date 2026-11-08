fn main() {
    let Ok(item) = std::env::var("DUNLIN_SSH_KEYCHAIN_ITEM") else {
        eprintln!("dunlin-ssh-askpass: DUNLIN_SSH_KEYCHAIN_ITEM is not set");
        std::process::exit(1);
    };
    match dunlin_ssh::keychain::get_passphrase(&item) {
        Ok(passphrase) => println!("{passphrase}"),
        Err(error) => {
            eprintln!("dunlin-ssh-askpass: {error}");
            std::process::exit(1);
        }
    }
}
