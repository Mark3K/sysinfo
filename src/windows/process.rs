// 
// Sysinfo
// 
// Copyright (c) 2018 Guillaume Gomez
//

use std::mem::{size_of, zeroed};
use std::fmt::{self, Formatter, Debug};
use std::str;

use libc::{c_uint, c_void, memcpy};

use Pid;
use ProcessExt;

use winapi::shared::minwindef::{DWORD, FALSE, FILETIME, MAX_PATH/*, TRUE, USHORT*/};
use winapi::um::handleapi::CloseHandle;
use winapi::um::winnt::{
    HANDLE, ULARGE_INTEGER, /*THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME,*/
    /*, PWSTR*/ PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE, PROCESS_VM_READ,
};
use winapi::um::processthreadsapi::{GetProcessTimes, OpenProcess, TerminateProcess};
use winapi::um::psapi::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    EnumProcessModulesEx, GetModuleBaseNameW, LIST_MODULES_ALL,
};
use winapi::um::sysinfoapi::GetSystemTimeAsFileTime;
use winapi::um::tlhelp32::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
};

/// Enum describing the different status of a process.
#[derive(Clone, Debug)]
pub enum ProcessStatus {
    /// Currently runnable.
    Run,
}

impl ProcessStatus {
    /// Used to display `ProcessStatus`.
    pub fn to_string(&self) -> &str {
        match *self {
            ProcessStatus::Run => "Runnable",
        }
    }
}

impl fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

fn get_process_handler(pid: Pid) -> Option<HANDLE> {
    if pid == 0 {
        return None;
    }
    let options = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_TERMINATE;
    let process_handler = unsafe { OpenProcess(options, FALSE, pid as DWORD) };
    if process_handler.is_null() {
        let options = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ;
        let process_handler = unsafe { OpenProcess(options, FALSE, pid as DWORD) };
        if process_handler.is_null() {
            None
        } else {
            Some(process_handler)
        }
    } else {
        Some(process_handler)
    }
}

/// Struct containing a process' information.
#[derive(Clone)]
pub struct Process {
    /// name of the program
    pub name: String,
    /// command line
    pub cmd: String,
    /// path to the executable
    pub exe: String,
    /// pid of the processus
    pub pid: Pid,
    /// Environment of the process.
    ///
    /// Always empty except for current process.
    pub environ: Vec<String>,
    /// current working directory
    pub cwd: String,
    /// path of the root directory
    pub root: String,
    /// memory usage (in kB)
    pub memory: u64,
    /// Parent pid.
    pub parent: Option<Pid>,
    /// Status of the Process.
    pub status: ProcessStatus,
    handle: HANDLE,
    old_cpu: u64,
    old_sys_cpu: u64,
    old_user_cpu: u64,
    /// time of process launch (in seconds)
    pub start_time: u64,
    /// total cpu usage
    pub cpu_usage: f32,
}

impl ProcessExt for Process {
    fn new(pid: Pid, parent: Option<Pid>, _: u64) -> Process {
        if let Some(process_handler) = get_process_handler(pid) {
            let mut h_mod = ::std::ptr::null_mut();
            let mut process_name = [0u16; MAX_PATH + 1];
            let mut cb_needed = 0;

            unsafe {
                if EnumProcessModulesEx(process_handler,
                                        &mut h_mod,
                                        ::std::mem::size_of::<DWORD>() as DWORD,
                                        &mut cb_needed,
                                        LIST_MODULES_ALL) != 0 {
                    GetModuleBaseNameW(process_handler,
                                       h_mod,
                                       process_name.as_mut_ptr(),
                                       MAX_PATH as DWORD + 1);
                }
                let mut pos = 0;
                for x in process_name.iter() {
                    if *x == 0 {
                        break
                    }
                    pos += 1;
                }
                let name = String::from_utf16_lossy(&process_name[..pos]);
                let environ = get_proc_env(process_handler, pid as u32, &name);
                Process {
                    handle: process_handler,
                    name: name,
                    pid: pid,
                    parent: parent,
                    cmd: get_cmd_line(process_handler),
                    environ: environ,
                    exe: String::new(),
                    cwd: String::new(),
                    root: String::new(),
                    status: ProcessStatus::Run,
                    memory: 0,
                    cpu_usage: 0.,
                    old_cpu: 0,
                    old_sys_cpu: 0,
                    old_user_cpu: 0,
                    start_time: get_start_time(process_handler),
                }
            }
        } else {
            Process {
                handle: ::std::ptr::null_mut(),
                name: String::new(),
                pid: pid,
                parent: parent,
                cmd: String::new(),
                environ: Vec::new(),
                exe: String::new(),
                cwd: String::new(),
                root: String::new(),
                status: ProcessStatus::Run,
                memory: 0,
                cpu_usage: 0.,
                old_cpu: 0,
                old_sys_cpu: 0,
                old_user_cpu: 0,
                start_time: 0,
            }
        }
    }

    fn kill(&self, signal: ::Signal) -> bool {
        let x = unsafe { TerminateProcess(self.handle, signal as c_uint) };
        println!("{:?} {:?} {:x}", self.handle, signal as c_uint, x);
        x != 0
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            if self.handle.is_null() {
                return
            }
            CloseHandle(self.handle);
        }
    }
}

#[allow(unused_must_use)]
impl Debug for Process {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "pid: {}\n", self.pid);
        write!(f, "name: {}\n", self.name);
        write!(f, "environment:\n");
        for var in self.environ.iter() {
        if var.len() > 0 {
                write!(f, "\t{}\n", var);
            }
        }
        write!(f, "command: {}\n", self.cmd);
        write!(f, "executable path: {}\n", self.exe);
        write!(f, "current working directory: {}\n", self.cwd);
        write!(f, "memory usage: {} kB\n", self.memory);
        write!(f, "cpu usage: {}%\n", self.cpu_usage);
        write!(f, "root path: {}", self.root)
    }
}

unsafe fn get_start_time(handle: HANDLE) -> u64 {
    let mut start = 0u64;
    let mut fstart = zeroed();
    let mut x = zeroed();

    GetProcessTimes(handle,
                    &mut fstart as *mut FILETIME,
                    &mut x as *mut FILETIME,
                    &mut x as *mut FILETIME,
                    &mut x as *mut FILETIME);
    memcpy(&mut start as *mut u64 as *mut c_void,
           &mut fstart as *mut FILETIME as *mut c_void,
           size_of::<FILETIME>());
    start
}

pub unsafe fn get_parent_process_id(pid: Pid) -> Option<Pid> {
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    let mut entry: PROCESSENTRY32 = zeroed();
    entry.dwSize = size_of::<PROCESSENTRY32>() as u32;
    let mut not_the_end = Process32First(snapshot, &mut entry);
    while not_the_end != 0 {
        if pid == entry.th32ProcessID as usize {
            // TODO: if some day I have the motivation to add threads:
            // ListProcessThreads(entry.th32ProcessID);
            CloseHandle(snapshot);
            return Some(entry.th32ParentProcessID as usize);
        }
        not_the_end = Process32Next(snapshot, &mut entry);
    }
    CloseHandle(snapshot);
    None
}

unsafe fn get_cmd_line(_handle: HANDLE) -> String {
    /*let mut pinfo: ffi::PROCESS_BASIC_INFORMATION = ::std::mem::zeroed();
    if ffi::NtQueryInformationProcess(handle,
                                           0, // ProcessBasicInformation
                                           &mut pinfo,
                                           size_of::<ffi::PROCESS_BASIC_INFORMATION>(),
                                           ::std::ptr::null_mut()) <= 0x7FFFFFFF {
        return String::new();
    }
    let ppeb: ffi::PPEB = pinfo.PebBaseAddress;
    let mut ppeb_copy: ffi::PEB = ::std::mem::zeroed();
    if kernel32::ReadProcessMemory(handle,
                                   ppeb as *mut raw::c_void,
                                   &mut ppeb_copy as *mut ffi::PEB as *mut raw::c_void,
                                   size_of::<ffi::PPEB>() as SIZE_T,
                                   ::std::ptr::null_mut()) != TRUE {
        return String::new();
    }

    let proc_param: ffi::PRTL_USER_PROCESS_PARAMETERS = ppeb_copy.ProcessParameters;
    let rtl_proc_param_copy: ffi::RTL_USER_PROCESS_PARAMETERS = ::std::mem::zeroed();
    if kernel32::ReadProcessMemory(handle,
                                   proc_param as *mut ffi::PRTL_USER_PROCESS_PARAMETERS *mut raw::c_void,
                                   &mut rtl_proc_param_copy as *mut ffi::RTL_USER_PROCESS_PARAMETERS as *mut raw::c_void,
                                   size_of::<ffi::RTL_USER_PROCESS_PARAMETERS>() as SIZE_T,
                                   ::std::ptr::null_mut()) != TRUE {
        return String::new();
    }
    let len: usize = rtl_proc_param_copy.CommandLine.Length as usize;
    let mut buffer_copy: Vec<u8> = Vec::with_capacity(len);
    buffer_copy.set_len(len);
    if kernel32::ReadProcessMemory(handle,
                                   rtl_proc_param_copy.CommandLine.Buffer as *mut raw::c_void,
                                   buffer_copy.as_mut_ptr() as *mut raw::c_void,
                                   len as SIZE_T,
                                   ::std::ptr::null_mut()) == TRUE {
        println!("{:?}", str::from_utf8_unchecked(buffer_copy.as_slice()));
        str::from_utf8_unchecked(buffer_copy.as_slice()).to_owned()
    } else {
        String::new()
    }*/
    String::new()
}

unsafe fn get_proc_env(_handle: HANDLE, _pid: u32, _name: &str) -> Vec<String> {
    let ret = Vec::new();
    /*
    println!("current pid: {}", kernel32::GetCurrentProcessId());
    if kernel32::GetCurrentProcessId() == pid {
        println!("current proc!");
        for (key, value) in env::vars() {
            ret.push(format!("{}={}", key, value));
        }
        return ret;
    }
    println!("1");
    let snapshot_handle = kernel32::CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
    if !snapshot_handle.is_null() {
        println!("2");
        let mut target_thread: THREADENTRY32 = zeroed();
        target_thread.dwSize = size_of::<THREADENTRY32>() as DWORD;
        if kernel32::Thread32First(snapshot_handle, &mut target_thread) == TRUE {
            println!("3");
            loop {
                if target_thread.th32OwnerProcessID == pid {
                    println!("4");
                    let thread_handle = kernel32::OpenThread(THREAD_SUSPEND_RESUME | THREAD_QUERY_INFORMATION | THREAD_GET_CONTEXT,
                                                             FALSE,
                                                             target_thread.th32ThreadID);
                    if !thread_handle.is_null() {
                        println!("5 -> {}", pid);
                        if kernel32::SuspendThread(thread_handle) != DWORD::max_value() {
                            println!("6");
                            let mut context = zeroed();
                            if kernel32::GetThreadContext(thread_handle, &mut context) != 0 {
                                println!("7 --> {:?}", context);
                                let mut x = vec![0u8; 10];
                                if kernel32::ReadProcessMemory(handle,
                                                               context.MxCsr as usize as *mut winapi::c_void,
                                                               x.as_mut_ptr() as *mut winapi::c_void,
                                                               x.len() as u64,
                                                               ::std::ptr::null_mut()) != 0 {
                                    for y in x {
                                        print!("{}", y as char);
                                    }
                                    println!("");
                                } else {
                                    println!("failure... {:?}", kernel32::GetLastError());
                                }
                            } else {
                                println!("-> {:?}", kernel32::GetLastError());
                            }
                            kernel32::ResumeThread(thread_handle);
                        }
                        kernel32::CloseHandle(thread_handle);
                    }
                    break;
                }
                if kernel32::Thread32Next(snapshot_handle, &mut target_thread) != TRUE {
                    break;
                }
            }
        }
        kernel32::CloseHandle(snapshot_handle);
    }*/
    ret
}

pub fn compute_cpu_usage(p: &mut Process, nb_processors: u64) {
    unsafe {
        let mut now: ULARGE_INTEGER = ::std::mem::zeroed();
        let mut sys: ULARGE_INTEGER = ::std::mem::zeroed();
        let mut user: ULARGE_INTEGER = ::std::mem::zeroed();
        let mut ftime: FILETIME = zeroed();
        let mut fsys: FILETIME = zeroed();
        let mut fuser: FILETIME = zeroed();

        GetSystemTimeAsFileTime(&mut ftime);
        memcpy(&mut now as *mut ULARGE_INTEGER as *mut c_void,
               &mut ftime as *mut FILETIME as *mut c_void,
               size_of::<FILETIME>());

        GetProcessTimes(p.handle,
                        &mut ftime as *mut FILETIME,
                        &mut ftime as *mut FILETIME,
                        &mut fsys as *mut FILETIME,
                        &mut fuser as *mut FILETIME);
        memcpy(&mut sys as *mut ULARGE_INTEGER as *mut c_void,
               &mut fsys as *mut FILETIME as *mut c_void,
               size_of::<FILETIME>());
        memcpy(&mut user as *mut ULARGE_INTEGER as *mut c_void,
               &mut fuser as *mut FILETIME as *mut c_void,
               size_of::<FILETIME>());
        p.cpu_usage = ((*sys.QuadPart() - p.old_sys_cpu) as f32 + (*user.QuadPart() - p.old_user_cpu) as f32)
            / (*now.QuadPart() - p.old_cpu) as f32 / nb_processors as f32 * 100.;
        p.old_cpu = *now.QuadPart();
        p.old_user_cpu = *user.QuadPart();
        p.old_sys_cpu = *sys.QuadPart();
    }
}

pub fn get_handle(p: &Process) -> HANDLE {
    p.handle
}

pub fn update_proc_info(p: &mut Process) {
    update_memory(p);
}

pub fn update_memory(p: &mut Process) {
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS_EX = zeroed();
        if GetProcessMemoryInfo(p.handle,
                                &mut pmc as *mut PROCESS_MEMORY_COUNTERS_EX as *mut c_void as *mut PROCESS_MEMORY_COUNTERS,
                                size_of::<PROCESS_MEMORY_COUNTERS_EX>() as DWORD) != 0 {
            p.memory = (pmc.PrivateUsage as u64) >> 10u64; // / 1024;
        }
    }
}