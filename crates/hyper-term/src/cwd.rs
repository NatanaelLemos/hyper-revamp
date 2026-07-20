use std::path::PathBuf;

/// Working directory of a live process, used for preserveCWD.
/// Returns None unless the path is absolute and still exists.
pub fn cwd_of_pid(pid: u32) -> Option<PathBuf> {
    let path = platform_cwd(pid)?;
    if path.is_absolute() && path.exists() {
        Some(path)
    } else {
        None
    }
}

/// libproc's `pidcwd` is a stub on macOS, so call
/// `proc_pidinfo(PROC_PIDVNODEPATHINFO)` directly — the same call the
/// Electron fork's `native-process-working-directory` module makes.
/// Struct layout from the SDK's `sys/proc_info.h`.
#[cfg(target_os = "macos")]
fn platform_cwd(pid: u32) -> Option<PathBuf> {
    use std::os::raw::{c_int, c_void};

    const MAXPATHLEN: usize = 1024;
    const PROC_PIDVNODEPATHINFO: c_int = 9;

    #[repr(C)]
    struct VinfoStat {
        vst_dev: u32,
        vst_mode: u16,
        vst_nlink: u16,
        vst_ino: u64,
        vst_uid: u32,
        vst_gid: u32,
        vst_atime: i64,
        vst_atimensec: i64,
        vst_mtime: i64,
        vst_mtimensec: i64,
        vst_ctime: i64,
        vst_ctimensec: i64,
        vst_birthtime: i64,
        vst_birthtimensec: i64,
        vst_size: i64,
        vst_blocks: i64,
        vst_blksize: i32,
        vst_flags: u32,
        vst_gen: u32,
        vst_rdev: u32,
        vst_qspare: [i64; 2],
    }

    #[repr(C)]
    struct VnodeInfo {
        vi_stat: VinfoStat,
        vi_type: c_int,
        vi_pad: c_int,
        vi_fsid: [i32; 2],
    }

    #[repr(C)]
    struct VnodeInfoPath {
        vip_vi: VnodeInfo,
        vip_path: [u8; MAXPATHLEN],
    }

    #[repr(C)]
    struct ProcVnodePathInfo {
        pvi_cdir: VnodeInfoPath,
        pvi_rdir: VnodeInfoPath,
    }

    extern "C" {
        fn proc_pidinfo(
            pid: c_int,
            flavor: c_int,
            arg: u64,
            buffer: *mut c_void,
            buffersize: c_int,
        ) -> c_int;
    }

    let mut info: ProcVnodePathInfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<ProcVnodePathInfo>() as c_int;
    let ret = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut ProcVnodePathInfo).cast(),
            size,
        )
    };
    if ret <= 0 {
        return None;
    }
    let path = &info.pvi_cdir.vip_path;
    let len = path.iter().position(|&b| b == 0)?;
    if len == 0 {
        return None;
    }
    Some(PathBuf::from(
        String::from_utf8_lossy(&path[..len]).into_owned(),
    ))
}

#[cfg(target_os = "linux")]
fn platform_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_of_spawned_child() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("5")
            .current_dir("/private/tmp")
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let cwd = cwd_of_pid(child.id());
        let _ = child.kill();
        assert_eq!(cwd, Some(PathBuf::from("/private/tmp")));
    }
}
