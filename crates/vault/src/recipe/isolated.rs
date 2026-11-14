//! Fixed-image Linux CLI execution. The host supplies an attested checkout FD;
//! no path from model arguments can select a mount or a program image.
use super::*;
use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{MetadataExt, OpenOptionsExt},
            process::CommandExt,
        },
    },
};
const BWRAP: &str = "/usr/bin/bwrap";
const IMAGE: &str = "/run/dunlin-recipe/program";
const MAX_IMAGE: u64 = 128 * 1024 * 1024;
pub struct LinuxRecipeExecutor {
    cwd: PathBuf,
    directory: File,
}
impl LinuxRecipeExecutor {
    /// Shape only: the caller's RecipeAuthority must independently attest that
    /// this exact directory descriptor belongs to its captured checkout.
    pub fn new(cwd: PathBuf, directory: File) -> Result<Self, String> {
        trusted_bwrap()?;
        if !cwd.is_absolute()
            || !directory
                .metadata()
                .map_err(|_| "Checkout unavailable")?
                .is_dir()
        {
            return Err("Invalid recipe checkout".into());
        }
        Ok(Self { cwd, directory })
    }
    fn prepare(&self, plan: &RecipePlan, access: &LiveAccess) -> Result<Command, String> {
        if !access.allowed()
            || self.cwd != plan.cwd
            || !dunlin_domain::vault::valid_recipe_secret_env(&plan.env_var)
        {
            return Err("Recipe target or environment is unavailable".into());
        }
        trusted_bwrap()?;
        let image = snapshot(plan, access)?;
        let image = inherited(&image)?;
        let root = inherited(&self.directory)?;
        let image_fd = image.as_raw_fd();
        let root_fd = root.as_raw_fd();
        let mut command = Command::new(BWRAP);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .env("TERM", "dumb")
            .env("NO_COLOR", "1")
            .env(&plan.env_var, plan.secret.as_str());
        command.args([
            "--die-with-parent",
            "--new-session",
            "--unshare-all",
            "--share-net",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
        ]);
        for path in ["/usr", "/bin", "/lib", "/lib64"] {
            if Path::new(path).exists() {
                command.args(["--ro-bind", path, path]);
            }
        }
        for path in [
            "/etc/resolv.conf",
            "/etc/hosts",
            "/etc/nsswitch.conf",
            "/etc/ssl",
            "/etc/pki",
        ] {
            if Path::new(path).exists() {
                command.args(["--ro-bind", path, path]);
            }
        }
        command
            .args([
                "--dir",
                "/home/dunlin",
                "--setenv",
                "HOME",
                "/home/dunlin",
                "--setenv",
                "XDG_CONFIG_HOME",
                "/home/dunlin/.config",
                "--setenv",
                "XDG_CACHE_HOME",
                "/home/dunlin/.cache",
                "--setenv",
                "XDG_DATA_HOME",
                "/home/dunlin/.local/share",
                "--setenv",
                "TMPDIR",
                "/tmp",
                "--dir",
                "/run/dunlin-recipe",
                "--perms",
                "0500",
                "--ro-bind-data",
            ])
            .arg(image_fd.to_string())
            .arg(IMAGE)
            .arg("--bind-fd")
            .arg(root_fd.to_string())
            .arg("/workspace")
            .args(["--chdir", "/workspace", "--", IMAGE])
            .args(&plan.args);
        // Only the operation's fork clears CLOEXEC. Other concurrent children
        // cannot inherit either descriptor. Captured Files keep them alive until
        // the command is dropped, including spawn retries.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(image.as_raw_fd(), libc::F_SETFD, 0) == -1
                    || libc::fcntl(root.as_raw_fd(), libc::F_SETFD, 0) == -1
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        if !access.allowed() {
            return Err("Recipe authority was revoked".into());
        }
        Ok(command)
    }
}
impl RecipeExecutor for LinuxRecipeExecutor {
    fn execute(&self, _: &RecipePlan) -> Result<RecipeOutput, String> {
        Err("Isolated recipes require live authority".into())
    }
    fn execute_guarded(
        &self,
        plan: &RecipePlan,
        access: Option<LiveAccess>,
    ) -> Result<RecipeOutput, String> {
        let access = access.ok_or("Isolated recipes require live authority")?;
        let command = self.prepare(plan, &access)?;
        run_command(command, plan.timeout.min(access.remaining()), Some(access))
    }
}
fn trusted_bwrap() -> Result<(), String> {
    // Root-owned, non-writable system paths form the trusted launcher boundary.
    for path in ["/", "/usr", "/usr/bin", BWRAP] {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| "Recipe sandbox unavailable")?;
        if metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
            || metadata.file_type().is_symlink()
            || (path == BWRAP && !metadata.is_file())
        {
            return Err("Untrusted recipe sandbox launcher".into());
        }
    }
    Ok(())
}
fn inherited(file: &File) -> Result<File, String> {
    let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 64) };
    if fd < 0 {
        return Err("Recipe descriptor capacity unavailable".into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn snapshot(plan: &RecipePlan, access: &LiveAccess) -> Result<File, String> {
    let expected = plan
        .program_hash
        .as_ref()
        .ok_or("Recipe program requires a pin")?;
    let program = std::fs::canonicalize(&plan.program).map_err(|_| "Recipe program unavailable")?;
    if program.starts_with(&plan.cwd) {
        return Err("Recipe program belongs to the checkout".into());
    }
    let mut source = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(program)
        .map_err(|_| "Recipe program unavailable")?;
    let meta = source
        .metadata()
        .map_err(|_| "Recipe program unavailable")?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_IMAGE {
        return Err("Invalid recipe program image".into());
    }
    let fd = unsafe {
        libc::memfd_create(
            c"dunlin-recipe".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if fd < 0 {
        return Err("Recipe image capacity unavailable".into());
    }
    let mut image = unsafe { File::from_raw_fd(fd) };
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buf = Zeroizing::new([0u8; 65536]);
    loop {
        if !access.allowed() {
            return Err("Recipe authority was revoked".into());
        }
        let n = source
            .read(&mut *buf)
            .map_err(|_| "Recipe image read failed")?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_IMAGE {
            return Err("Recipe image exceeds its limit".into());
        }
        hash.update(&buf[..n]);
        image
            .write_all(&buf[..n])
            .map_err(|_| "Recipe image write failed")?;
    }
    if total == 0 || !hex::encode(hash.finalize()).eq_ignore_ascii_case(expected) {
        return Err("Recipe program changed".into());
    }
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, seals) } < 0 {
        return Err("Recipe image could not be sealed".into());
    }
    image
        .seek(SeekFrom::Start(0))
        .map_err(|_| "Recipe image unavailable")?;
    Ok(image)
}

#[cfg(test)]
mod tests;
