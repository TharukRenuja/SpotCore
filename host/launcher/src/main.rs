use std::{
    ffi::OsString,
    process::{Command, ExitCode},
};

#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Job {
    fn attach(child: &std::process::Child) -> std::io::Result<Self> {
        use std::{
            mem::{size_of, zeroed},
            os::windows::io::AsRawHandle,
        };
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = Self(CreateJobObjectW(std::ptr::null(), std::ptr::null()));
            if job.0.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
                || AssignProcessToJobObject(job.0, child.as_raw_handle()) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(unix)]
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(unix)]
extern "C" fn interrupted(_: libc::c_int) {
    STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}

struct Owner {
    #[cfg(not(windows))]
    pid: u32,
    #[cfg(windows)]
    handle: std::os::windows::io::OwnedHandle,
}
impl Owner {
    fn new(pid: u32) -> Result<Self, &'static str> {
        if pid == 0 {
            return Err("invalid owner pid");
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::FromRawHandle;
            use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            if handle.is_null() {
                return Err("could not watch launcher owner");
            }
            Ok(Self {
                handle: unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) },
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self { pid })
        }
    }
    fn alive(&self) -> bool {
        #[cfg(unix)]
        {
            unsafe { libc::getppid() as u32 == self.pid }
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(
                    self.handle.as_raw_handle(),
                    0,
                ) == windows_sys::Win32::Foundation::WAIT_TIMEOUT
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = self.pid;
            true
        }
    }
}

fn run() -> Result<u8, &'static str> {
    let mut args = std::env::args_os().skip(1);
    let mut first = args.next();
    let owner = if first.as_deref() == Some(std::ffi::OsStr::new("--parent-pid")) {
        let pid = args
            .next()
            .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
            .ok_or("invalid owner pid")?;
        first = args.next();
        Some(Owner::new(pid)?)
    } else {
        None
    };
    if first.as_deref() != Some(std::ffi::OsStr::new("--backend")) {
        return Err("usage: spotcore-engine --backend <executable> -- [backend arguments]");
    }
    if owner.as_ref().is_some_and(|owner| !owner.alive()) {
        return Err("launcher owner is unavailable");
    }
    let backend: OsString = args
        .next()
        .filter(|s| !s.is_empty())
        .ok_or("backend executable required")?;
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
        return Err("backend argument separator required");
    }
    let backend = std::fs::canonicalize(backend).map_err(|_| "backend executable was not found")?;
    let current = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| "could not resolve launcher")?;
    if backend == current {
        return Err("launcher cannot be its own backend");
    }
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_NAME, c"spotcore-engine".as_ptr(), 0, 0, 0);
    }
    let mut backend_args: Vec<_> = args.collect();
    for index in 0..backend_args.len() {
        if backend_args[index] == "--parent-pid" {
            let value = backend_args
                .get_mut(index + 1)
                .ok_or("backend parent pid missing")?;
            *value = std::process::id().to_string().into();
        }
    }
    #[cfg(unix)]
    unsafe {
        if owner.is_some() {
            libc::setpgid(0, 0);
        }
        libc::signal(
            libc::SIGTERM,
            interrupted as *const () as libc::sighandler_t,
        );
        libc::signal(libc::SIGINT, interrupted as *const () as libc::sighandler_t);
    }
    let mut child = Command::new(backend)
        .args(backend_args)
        .spawn()
        .map_err(|_| "could not start backend")?;
    #[cfg(windows)]
    let _job = match Job::attach(&child) {
        Ok(job) => job,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("could not supervise backend process tree");
        }
    };
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| "could not wait for backend")? {
            break status;
        }
        #[cfg(unix)]
        let stopped = STOP.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(unix))]
        let stopped = false;
        if stopped || owner.as_ref().is_some_and(|owner| !owner.alive()) {
            #[cfg(unix)]
            unsafe {
                if libc::getpgrp() == std::process::id() as libc::pid_t {
                    libc::killpg(libc::getpgrp(), libc::SIGKILL);
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err("launcher owner exited or shutdown requested");
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    };
    Ok(status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1))
}
fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
