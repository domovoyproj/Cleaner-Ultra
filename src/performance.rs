//! Read-only resource monitor and explicit native Windows power-plan selection.
use anyhow::{Result, bail};
use serde_json::{Value, json};

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        collections::HashMap,
        ffi::c_void,
        mem::size_of,
        ptr::{null, null_mut},
        sync::{Mutex, OnceLock},
    };
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }
    impl Guid {
        fn text(self) -> String {
            format!(
                "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                self.a,
                self.b,
                self.c,
                self.d[0],
                self.d[1],
                self.d[2],
                self.d[3],
                self.d[4],
                self.d[5],
                self.d[6],
                self.d[7]
            )
        }
    }
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    impl FileTime {
        fn value(&self) -> u64 {
            (self.high as u64) << 32 | self.low as u64
        }
    }
    #[repr(C)]
    #[derive(Default)]
    struct Memory {
        length: u32,
        load: u32,
        total: u64,
        available: u64,
        page_total: u64,
        page_available: u64,
        virtual_total: u64,
        virtual_available: u64,
        extended: u64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak: usize,
        working: usize,
        pool_peak: usize,
        pool: usize,
        nonpaged_peak: usize,
        nonpaged: usize,
        pagefile: usize,
        pagefile_peak: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn GlobalMemoryStatusEx(memory: *mut Memory) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn QueryFullProcessImageNameW(
            handle: *mut c_void,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn GetProcessTimes(
            handle: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn EnumProcesses(pids: *mut u32, size: u32, needed: *mut u32) -> i32;
        fn GetProcessMemoryInfo(process: *mut c_void, counters: *mut Counters, size: u32) -> i32;
    }
    #[link(name = "powrprof")]
    unsafe extern "system" {
        fn PowerGetActiveScheme(root: *mut c_void, scheme: *mut *mut Guid) -> u32;
        fn PowerEnumerate(
            root: *mut c_void,
            scheme: *const Guid,
            subgroup: *const Guid,
            access: u32,
            index: u32,
            buffer: *mut u8,
            size: *mut u32,
        ) -> u32;
        fn PowerReadFriendlyName(
            root: *mut c_void,
            scheme: *const Guid,
            subgroup: *const Guid,
            setting: *const Guid,
            buffer: *mut u8,
            size: *mut u32,
        ) -> u32;
        fn PowerSetActiveScheme(root: *mut c_void, scheme: *const Guid) -> u32;
    }
    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    #[derive(Default)]
    struct Sample {
        total: u64,
        idle: u64,
        processes: HashMap<(u32, u64), u64>,
    }
    static SAMPLE: OnceLock<Mutex<Sample>> = OnceLock::new();
    fn check(code: u32) -> Result<()> {
        if code == 0 {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(code as i32).into())
        }
    }
    pub fn resources() -> Result<Value> {
        unsafe {
            let mut last = SAMPLE
                .get_or_init(|| Mutex::new(Sample::default()))
                .lock()
                .map_err(|_| anyhow::anyhow!("Монитор занят"))?;
            let (mut idle, mut kernel, mut user) = (
                FileTime::default(),
                FileTime::default(),
                FileTime::default(),
            );
            if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let total = kernel.value().saturating_add(user.value());
            let delta = total.saturating_sub(last.total);
            let cpu = if last.total > 0 && delta > 0 {
                Some(
                    (100.0 * (delta.saturating_sub(idle.value().saturating_sub(last.idle))) as f64
                        / delta as f64)
                        .clamp(0.0, 100.0),
                )
            } else {
                None
            };
            let mut memory = Memory {
                length: size_of::<Memory>() as u32,
                ..Default::default()
            };
            if GlobalMemoryStatusEx(&mut memory) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut pids = vec![0u32; 65536];
            let mut needed = 0;
            if EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut needed) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut processes = Vec::new();
            let mut samples = HashMap::new();
            let mut inaccessible = 0;
            for pid in pids
                .into_iter()
                .take(needed as usize / 4)
                .filter(|p| *p != 0)
            {
                let handle = OpenProcess(0x1000 | 0x10, 0, pid);
                if handle.is_null() {
                    inaccessible += 1;
                    continue;
                }
                let handle = Handle(handle);
                let mut name = vec![0u16; 32768];
                let mut count = name.len() as u32;
                if QueryFullProcessImageNameW(handle.0, 0, name.as_mut_ptr(), &mut count) == 0 {
                    inaccessible += 1;
                    continue;
                }
                let path = String::from_utf16_lossy(&name[..count as usize]);
                let name = path.rsplit('\\').next().unwrap_or(&path);
                let mut counters = Counters {
                    cb: size_of::<Counters>() as u32,
                    ..Default::default()
                };
                if GetProcessMemoryInfo(handle.0, &mut counters, size_of::<Counters>() as u32) == 0
                {
                    inaccessible += 1;
                    continue;
                }
                let (mut creation, mut exit, mut pk, mut pu) = (
                    FileTime::default(),
                    FileTime::default(),
                    FileTime::default(),
                    FileTime::default(),
                );
                let cpu =
                    if GetProcessTimes(handle.0, &mut creation, &mut exit, &mut pk, &mut pu) != 0 {
                        let t = pk.value().saturating_add(pu.value());
                        let key = (pid, creation.value());
                        samples.insert(key, t);
                        last.processes
                            .get(&key)
                            .filter(|_| delta > 0)
                            .map(|before| {
                                (100.0 * t.saturating_sub(*before) as f64 / delta as f64)
                                    .clamp(0.0, 100.0)
                            })
                    } else {
                        None
                    };
                processes.push(json!({"pid":pid,"name":name,"memory_bytes":counters.working,"cpu_percent":cpu}));
            }
            *last = Sample {
                total,
                idle: idle.value(),
                processes: samples,
            };
            processes.sort_by_key(|p| std::cmp::Reverse(p["memory_bytes"].as_u64().unwrap_or(0)));
            Ok(
                json!({"cpu_percent":cpu,"memory_percent":memory.load,"memory_total":memory.total,"memory_available":memory.available,"processes":processes,"inaccessible_processes":inaccessible}),
            )
        }
    }
    fn schemes() -> Result<(String, Vec<(Guid, String)>)> {
        unsafe {
            let mut active = null_mut();
            check(PowerGetActiveScheme(null_mut(), &mut active))?;
            if active.is_null() {
                bail!("Не получена активная схема");
            }
            let active_id = (*active).text();
            LocalFree(active.cast());
            let mut schemes = Vec::new();
            for index in 0..256 {
                let mut guid = Guid::default();
                let mut size = size_of::<Guid>() as u32;
                let code = PowerEnumerate(
                    null_mut(),
                    null(),
                    null(),
                    16,
                    index,
                    (&mut guid as *mut Guid).cast(),
                    &mut size,
                );
                if code == 259 {
                    return Ok((active_id, schemes));
                }
                check(code)?;
                let mut size = 0;
                let code =
                    PowerReadFriendlyName(null_mut(), &guid, null(), null(), null_mut(), &mut size);
                if code != 0 && code != 234 {
                    check(code)?;
                }
                if size > 65536 {
                    bail!("Имя схемы слишком длинное");
                }
                let mut name = vec![0u16; (size as usize).div_ceil(2).max(1)];
                check(PowerReadFriendlyName(
                    null_mut(),
                    &guid,
                    null(),
                    null(),
                    name.as_mut_ptr().cast(),
                    &mut size,
                ))?;
                let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
                schemes.push((guid, String::from_utf16_lossy(&name[..len])));
            }
            bail!("Слишком много схем питания")
        }
    }
    pub fn power() -> Result<Value> {
        let (active, plans) = schemes()?;
        Ok(
            json!({"active":active,"plans":plans.into_iter().map(|(g,name)|json!({"id":g.text(),"name":name})).collect::<Vec<_>>()}),
        )
    }
    pub fn set_power(id: &str, backup: impl FnOnce(&str) -> Result<()>) -> Result<Value> {
        let (previous, plans) = schemes()?;
        let (guid, name) = plans
            .into_iter()
            .find(|(g, _)| g.text() == id)
            .ok_or_else(|| anyhow::anyhow!("Выберите существующую схему питания"))?;
        backup(&previous)?;
        unsafe {
            check(PowerSetActiveScheme(null_mut(), &guid))?;
        }
        let current = power()?;
        if current["active"] != id {
            bail!("Windows не применила схему");
        }
        Ok(json!({"done":1,"previous":previous,"active":id,"name":name,"errors":[]}))
    }
}
#[cfg(windows)]
pub use native::{power, resources, set_power};
#[cfg(not(windows))]
pub fn resources() -> Result<Value> {
    bail!("Монитор доступен на Windows")
}
#[cfg(not(windows))]
pub fn power() -> Result<Value> {
    bail!("Схемы питания доступны на Windows")
}
#[cfg(not(windows))]
pub fn set_power(_: &str, _: impl FnOnce(&str) -> Result<()>) -> Result<Value> {
    bail!("Схемы питания доступны на Windows")
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn failed_durable_backup_prevents_power_change() {
        let before = super::power().unwrap();
        let id = before["active"].as_str().unwrap();
        assert!(super::set_power(id, |_| anyhow::bail!("simulated journal failure")).is_err());
        assert_eq!(super::power().unwrap()["active"], id);
    }
}
