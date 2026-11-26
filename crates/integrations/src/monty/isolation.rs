//! Kernel boundary applied before executing Monty. Linux only, fail closed.
//! Landlock permits reading shared libraries and executing the pinned runtime;
//! seccomp denies networking, new processes, namespace and inspection APIs.
use std::{
    ffi::CString,
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
};

fn check(value: libc::c_long) -> io::Result<libc::c_long> {
    if value < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(value)
    }
}
#[repr(C)]
struct Ruleset {
    handled_access_fs: u64,
}
#[repr(C, packed)]
struct PathRule {
    allowed_access: u64,
    parent_fd: i32,
}
const EXECUTE: u64 = 1;
const READ_FILE: u64 = 1 << 2;
const READ_DIR: u64 = 1 << 3;

pub(super) fn restrict(runtime: &Path) -> io::Result<()> {
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return Err(io::Error::other("unsupported Monty sandbox architecture"));
    // SAFETY: all syscall pointers refer to live C-layout values with the
    // documented lengths; returned descriptors are transferred to OwnedFd.
    unsafe {
        check(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) as _)?;
        check(libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) as _)?;
        let abi = check(libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<Ruleset>(),
            0usize,
            1u32,
        ))?;
        if abi < 3 {
            return Err(io::Error::other("Landlock ABI 3 or newer required"));
        }
        // Through TRUNCATE (ABI 3). Newer kernels' extra permissions stay
        // unhandled; seccomp separately denies device/ioctl and networking.
        let attr = Ruleset {
            handled_access_fs: (1 << 15) - 1,
        };
        let fd = check(libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr,
            std::mem::size_of::<Ruleset>(),
            0u32,
        ))? as i32;
        let ruleset = OwnedFd::from_raw_fd(fd);
        allow_path(&ruleset, runtime, EXECUTE | READ_FILE)?;
        for path in ["/lib", "/lib64", "/usr/lib", "/usr/lib64"] {
            if Path::new(path).exists() {
                allow_path(&ruleset, Path::new(path), READ_FILE | READ_DIR)?;
            }
        }
        check(libc::syscall(
            libc::SYS_landlock_restrict_self,
            ruleset.as_raw_fd(),
            0u32,
        ))?;
        for (resource, value) in [
            (libc::RLIMIT_AS, 512 * 1024 * 1024),
            (libc::RLIMIT_CPU, 15),
            (libc::RLIMIT_CORE, 0),
            (libc::RLIMIT_FSIZE, 0),
            (libc::RLIMIT_NOFILE, 64),
        ] {
            let limit = libc::rlimit {
                rlim_cur: value,
                rlim_max: value,
            };
            check(libc::setrlimit(resource, &limit) as _)?;
        }
        // Keep only the three protocol/diagnostic streams across exec.
        check(libc::syscall(
            libc::SYS_close_range,
            3u32,
            u32::MAX,
            1u32 << 2,
        ))?;
        seccomp()?;
    }
    Ok(())
}

unsafe fn allow_path(ruleset: &OwnedFd, path: &Path, access: u64) -> io::Result<()> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::other("invalid runtime path"))?;
    // SAFETY: the path is NUL-terminated and the attribute lives through add_rule.
    unsafe {
        let fd = OwnedFd::from_raw_fd(check(libc::open(
            path.as_ptr(),
            libc::O_PATH | libc::O_CLOEXEC,
        ) as _)? as i32);
        let attr = PathRule {
            allowed_access: access,
            parent_fd: fd.as_raw_fd(),
        };
        check(libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset.as_raw_fd(),
            1u32,
            &attr,
            0u32,
        ))?;
    }
    Ok(())
}

unsafe fn seccomp() -> io::Result<()> {
    fn statement(code: u16, k: u32) -> libc::sock_filter {
        libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }
    fn equal(k: u32, jt: u8, jf: u8) -> libc::sock_filter {
        libc::sock_filter {
            code: 0x15,
            jt,
            jf,
            k,
        }
    }
    #[cfg(target_arch = "x86_64")]
    let architecture = 0xc000_003e;
    #[cfg(target_arch = "aarch64")]
    let architecture = 0xc000_00b7;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let architecture = 0;
    let mut filter = vec![
        statement(0x20, 4), // load seccomp_data.arch
        equal(architecture, 1, 0),
        statement(0x06, 0x8000_0000), // KILL_PROCESS on an alternate ABI
        statement(0x20, 0),           // load seccomp_data.nr
    ];
    // x32 shares AUDIT_ARCH_X86_64; reject its alternate syscall numbering.
    #[cfg(target_arch = "x86_64")]
    filter.extend([
        libc::sock_filter {
            code: 0x35,
            jt: 0,
            jf: 1,
            k: 0x4000_0000,
        },
        statement(0x06, 0x8000_0000),
    ]);
    // glibc retries with legacy clone only on ENOSYS. Monty starts one
    // interpreter thread; allow pthread-style shared-address-space clones,
    // never a process or a new namespace. Inspect both halves of flags.
    filter.extend([
        equal(libc::SYS_clone3 as u32, 0, 1),
        statement(0x06, 0x0005_0000 | libc::ENOSYS as u32),
    ]);
    let required = (libc::CLONE_VM | libc::CLONE_SIGHAND | libc::CLONE_THREAD) as u32;
    let allowed = required
        | (libc::CLONE_FS
            | libc::CLONE_FILES
            | libc::CLONE_SYSVSEM
            | libc::CLONE_SETTLS
            | libc::CLONE_PARENT_SETTID
            | libc::CLONE_CHILD_CLEARTID
            | libc::CLONE_CHILD_SETTID
            | libc::CLONE_DETACHED) as u32;
    let deny = || statement(0x06, 0x0005_0000 | libc::EPERM as u32);
    filter.extend([
        equal(libc::SYS_clone as u32, 0, 12),
        statement(0x20, 20), // high half of args[0]
        equal(0, 1, 0),
        deny(),
        statement(0x20, 16),       // low half of args[0]
        statement(0x54, !allowed), // AND
        equal(0, 1, 0),
        deny(),
        statement(0x20, 16),
        statement(0x54, required),
        equal(required, 0, 1),
        statement(0x06, 0x7fff_0000),
        deny(),
    ]);
    let mut denied = vec![
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_sendto,
        libc::SYS_sendmsg,
        libc::SYS_sendmmsg,
        libc::SYS_recvmsg,
        libc::SYS_recvmmsg,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_pidfd_getfd,
        libc::SYS_pidfd_open,
        libc::SYS_kill,
        libc::SYS_tkill,
        libc::SYS_tgkill,
        libc::SYS_pidfd_send_signal,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_io_uring_setup,
        libc::SYS_ioctl,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
    ];
    #[cfg(target_arch = "x86_64")]
    denied.extend([libc::SYS_fork, libc::SYS_vfork]);
    for syscall in denied {
        filter.push(equal(syscall as u32, 0, 1));
        filter.push(statement(0x06, 0x0005_0000 | libc::EPERM as u32));
    }
    filter.push(statement(0x06, 0x7fff_0000)); // ALLOW remaining calls; Landlock mediates paths
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };
    // SAFETY: the kernel copies the bounded filter before returning.
    unsafe {
        check(libc::prctl(libc::PR_SET_SECCOMP, 2, &program, 0, 0) as _)?;
    }
    Ok(())
}

#[cfg(all(test, feature = "monty-isolation-tests"))]
mod tests {
    #[test]
    fn monty_isolation_child() {
        if std::env::var_os("EVERRUNS_MONTY_ISOLATION_PROBE").is_none() {
            return;
        }
        let executable = std::env::current_exe().unwrap();
        super::restrict(&executable).expect("Linux host must support Landlock ABI 3 and seccomp");
        assert!(std::fs::read(&executable).is_ok());
        assert!(std::fs::read("/etc/passwd").is_err());
        assert!(std::fs::write("/tmp/monty-isolation-escape", b"blocked").is_err());
        assert!(std::net::TcpStream::connect("127.0.0.1:9").is_err());
        // SAFETY: these calls must be refused by the installed filter.
        unsafe {
            assert_eq!(libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0), -1);
            assert_eq!(
                libc::syscall(libc::SYS_clone3, std::ptr::null::<u8>(), 0),
                -1
            );
            assert_eq!(
                libc::syscall(libc::SYS_io_uring_setup, 0, std::ptr::null::<u8>()),
                -1
            );
        }
    }
    #[test]
    fn monty_kernel_boundary_denies_host_access() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "isolation::tests::monty_isolation_child",
                "--nocapture",
            ])
            .env("EVERRUNS_MONTY_ISOLATION_PROBE", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn monty_seccomp_child() {
        if std::env::var_os("EVERRUNS_MONTY_SECCOMP_PROBE").is_none() {
            return;
        }
        // SAFETY: restrict only this disposable test process.
        unsafe {
            super::check(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) as _).unwrap();
            super::seccomp().unwrap();
        }
        assert_eq!(std::thread::spawn(|| 7).join().unwrap(), 7);
        assert!(std::process::Command::new("/bin/true").status().is_err());
        assert!(std::net::TcpStream::connect("127.0.0.1:9").is_err());
        // Thread creation may not smuggle a new namespace or a process.
        unsafe {
            assert_eq!(
                libc::syscall(libc::SYS_clone, libc::SIGCHLD, 0, 0, 0, 0),
                -1
            );
            assert_eq!(
                libc::syscall(
                    libc::SYS_clone,
                    libc::CLONE_VM | libc::CLONE_THREAD | libc::CLONE_SIGHAND | libc::CLONE_NEWUSER,
                    0,
                    0,
                    0,
                    0
                ),
                -1
            );
        }
    }
    #[test]
    fn monty_seccomp_allows_worker_threads_but_denies_processes() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "isolation::tests::monty_seccomp_child",
                "--nocapture",
            ])
            .env("EVERRUNS_MONTY_SECCOMP_PROBE", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
