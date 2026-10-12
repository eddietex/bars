//! Finds Claude Code or Codex running below a terminal's process (in any
//! of its tabs), for terminals whose window title doesn't name them.

use std::collections::{HashMap, HashSet, VecDeque};

use super::focus::agent_from_process;
use crate::text::Agent;

/// Breadth-first through the descendants of `root`.
fn walk(root: u32, children: &HashMap<u32, Vec<u32>>, check: impl Fn(u32) -> Option<Agent>) -> Option<Agent> {
    let mut queue = VecDeque::from([root]);
    // Process ids get reused, so parent links can form cycles.
    let mut seen = HashSet::from([root]);
    while let Some(pid) = queue.pop_front() {
        for &child in children.get(&pid).into_iter().flatten() {
            if !seen.insert(child) || seen.len() > 4096 {
                continue;
            }
            if let Some(agent) = check(child) {
                return Some(agent);
            }
            queue.push_back(child);
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub fn agent_below(root: u32) -> Option<Agent> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
        // "pid (comm) state ppid ...", where comm may hold spaces and parentheses.
        let Some(rest) = stat.rfind(')').map(|i| &stat[i + 1..]) else { continue };
        if let Some(ppid) = rest.split_whitespace().nth(1).and_then(|s| s.parse::<u32>().ok()) {
            children.entry(ppid).or_default().push(pid);
        }
    }
    walk(root, &children, linux_agent)
}

/// An agent running anywhere in this session.
#[cfg(target_os = "linux")]
pub fn agent_running() -> Option<Agent> {
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
        let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
        linux_agent(pid)
    })
}

#[cfg(target_os = "linux")]
fn linux_agent(pid: u32) -> Option<Agent> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    agent_from_process(name.trim(), || {
        std::fs::read(format!("/proc/{pid}/cmdline"))
            .map(|b| b.split(|c| *c == 0).map(|s| String::from_utf8_lossy(s).into_owned()).collect())
            .unwrap_or_default()
    })
}

#[cfg(not(target_os = "linux"))]
pub fn agent_running() -> Option<Agent> {
    None
}

#[cfg(target_os = "macos")]
pub fn agent_below(root: u32) -> Option<Agent> {
    use libc::{PROC_PIDT_SHORTBSDINFO, c_void, proc_bsdshortinfo, proc_listallpids, proc_pidinfo};

    let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return None;
    }
    let mut pids = vec![0i32; count as usize + 64];
    let bytes = (pids.len() * size_of::<i32>()) as i32;
    let count = unsafe { proc_listallpids(pids.as_mut_ptr() as *mut c_void, bytes) };
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut names = HashMap::new();
    for &pid in pids.iter().take(count.max(0) as usize) {
        // The short form works for other users' processes too (Terminal's
        // `login` is root's).
        let mut info: proc_bsdshortinfo = unsafe { std::mem::zeroed() };
        let size = size_of::<proc_bsdshortinfo>() as i32;
        let got = unsafe { proc_pidinfo(pid, PROC_PIDT_SHORTBSDINFO, 0, &mut info as *mut _ as *mut c_void, size) };
        if got != size {
            continue;
        }
        children.entry(info.pbsi_ppid).or_default().push(pid as u32);
        let comm: Vec<u8> = info.pbsi_comm.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        names.insert(pid as u32, String::from_utf8_lossy(&comm).into_owned());
    }
    walk(root, &children, |pid| agent_from_process(names.get(&pid)?, || mac_args(pid)))
}

/// A process's arguments (KERN_PROCARGS2: argc, the executable path, then
/// the NUL-separated arguments).
#[cfg(target_os = "macos")]
fn mac_args(pid: u32) -> Vec<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
    let mut size: usize = 0;
    let null = std::ptr::null_mut();
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, null, &mut size, null, 0) } != 0 || size < 4 {
        return Vec::new();
    }
    let mut buf = vec![0u8; size];
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, null, 0) } != 0 {
        return Vec::new();
    }
    buf.truncate(size);
    let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]).max(0) as usize;
    let rest = &buf[4..];
    // Skip the executable path and the padding after it.
    let start = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
    let start = rest[start..].iter().position(|b| *b != 0).map_or(rest.len(), |p| start + p);
    rest[start..]
        .split(|b| *b == 0)
        .take(argc)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

#[cfg(target_os = "windows")]
pub fn agent_below(root: u32) -> Option<Agent> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };

    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut names = HashMap::new();
    let mut more = unsafe { Process32FirstW(snap, &mut entry) }.is_ok();
    while more {
        let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
        names.insert(entry.th32ProcessID, String::from_utf16_lossy(&entry.szExeFile[..len]));
        children.entry(entry.th32ParentProcessID).or_default().push(entry.th32ProcessID);
        more = unsafe { Process32NextW(snap, &mut entry) }.is_ok();
    }
    unsafe {
        let _ = CloseHandle(snap);
    }
    // Another process's command line isn't readable without peeking into
    // its memory, so a Node-launched agent is only found by its title.
    walk(root, &children, |pid| agent_from_process(names.get(&pid)?, Vec::new))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn agent_below(_root: u32) -> Option<Agent> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn finds_an_agent_among_descendants() {
        let children = HashMap::from([(1, vec![2, 3]), (3, vec![4]), (4, vec![1])]);
        let found = walk(1, &children, |pid| (pid == 4).then_some(Agent::Codex));
        assert_eq!(found, Some(Agent::Codex));
        assert_eq!(walk(1, &children, |_| None), None);
    }
}
