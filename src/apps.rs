//! Installed Win32 applications. Launch the vendor's registered uninstaller;
//! never delete application directories or guess remaining registry keys.
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
#[derive(Clone, Serialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub size_kb: u64,
    pub removable: bool,
    #[serde(skip)]
    pub command: String,
    #[serde(skip)]
    pub product: String,
}
pub struct Apps {
    snapshot: Mutex<HashMap<String, App>>,
}
impl Default for Apps {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(HashMap::new()),
        }
    }
}
impl Apps {
    pub fn list(&self) -> Result<Value> {
        let (mut items, warnings) = inventory()?;
        items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        *self.snapshot.lock() = items.iter().map(|a| (a.id.clone(), a.clone())).collect();
        Ok(json!({"items":items,"warnings":warnings,"scope":"Win32 · HKCU/HKLM · 32/64-bit"}))
    }
    pub fn uninstall(&self, id: &str) -> Result<Value> {
        let app = self
            .snapshot
            .lock()
            .get(id)
            .cloned()
            .context("Обновите список приложений")?;
        let (items, _) = inventory()?;
        let current = items
            .iter()
            .find(|a| a.id == id)
            .context("Приложение уже удалено или запись изменилась")?;
        if !app.removable || current.command != app.command || current.product != app.product {
            bail!("Запись удаления изменилась. Обновите список");
        }
        launch(&app)?;
        Ok(
            json!({"launched":true,"name":app.name,"message":"Запущен деинсталлятор. Завершите его работу и обновите список для проверки."}),
        )
    }
}
fn guid(s: &str) -> bool {
    s.len() == 38
        && s.starts_with('{')
        && s.ends_with('}')
        && s.as_bytes().iter().enumerate().all(|(i, b)| match i {
            0 => *b == b'{',
            37 => *b == b'}',
            9 | 14 | 19 | 24 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}
fn command_parts(s: &str) -> Option<(String, String)> {
    if s.chars().any(char::is_control) {
        return None;
    }
    let text = s.trim();
    let (path, args) = if let Some(q) = text.strip_prefix('"') {
        let (p, a) = q.split_once('"')?;
        if !a.is_empty() && !a.starts_with(char::is_whitespace) {
            return None;
        }
        (p, a)
    } else {
        text.split_once(char::is_whitespace).unwrap_or((text, ""))
    };
    let executable = crate::maintenance::executable(&crate::space::quote_windows_arg(path))?;
    let name = path.rsplit('\\').next()?.to_ascii_lowercase();
    if ["msiexec.exe", "explorer.exe", "reg.exe", "schtasks.exe"].contains(&name.as_str()) {
        return None;
    }
    Some((executable, args.trim().into()))
}
#[cfg(windows)]
fn launch(app: &App) -> Result<()> {
    use std::{os::windows::process::CommandExt, path::PathBuf, process::Command};
    if guid(&app.product) {
        let exe = PathBuf::from(std::env::var_os("SystemRoot").context("Нет Windows")?)
            .join("System32/msiexec.exe");
        Command::new(exe).args(["/x", &app.product]).spawn()?;
    } else {
        let (exe, args) = command_parts(&app.command)
            .context("Неоднозначная команда удаления. Используйте параметры Windows")?;
        if !PathBuf::from(&exe).is_file() {
            bail!("Деинсталлятор не найден");
        }
        let result = Command::new(&exe)
            .raw_arg(&args)
            .current_dir(PathBuf::from(&exe).parent().context("Нет папки")?)
            .spawn();
        if let Err(error) = result {
            if error.raw_os_error() != Some(740) {
                return Err(error.into());
            }
            #[link(name = "shell32")]
            unsafe extern "system" {
                fn ShellExecuteW(
                    hwnd: *mut std::ffi::c_void,
                    operation: *const u16,
                    file: *const u16,
                    params: *const u16,
                    directory: *const u16,
                    show: i32,
                ) -> isize;
            }
            let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let result = unsafe {
                ShellExecuteW(
                    std::ptr::null_mut(),
                    wide("runas").as_ptr(),
                    wide(&exe).as_ptr(),
                    wide(&args).as_ptr(),
                    std::ptr::null(),
                    1,
                )
            };
            if result <= 32 {
                bail!("Запуск деинсталлятора отменён или недоступен ({result})");
            }
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn launch(_: &App) -> Result<()> {
    bail!("Удаление программ доступно на Windows")
}
#[cfg(not(windows))]
fn inventory() -> Result<(Vec<App>, Vec<String>)> {
    bail!("Приложения доступны на Windows")
}
#[cfg(windows)]
fn inventory() -> Result<(Vec<App>, Vec<String>)> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS},
        System::Registry::*,
    };
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn read(key: HKEY, name: &str) -> Option<(u32, Vec<u8>)> {
        let mut kind = 0;
        let mut size = 0;
        if unsafe {
            RegQueryValueExW(
                key,
                wide(name).as_ptr(),
                null(),
                &mut kind,
                null_mut(),
                &mut size,
            )
        } != 0
            || size > 1024 * 1024
        {
            return None;
        }
        let mut data = vec![0; size as usize];
        if unsafe {
            RegQueryValueExW(
                key,
                wide(name).as_ptr(),
                null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut size,
            )
        } != 0
        {
            return None;
        }
        data.truncate(size as usize);
        Some((kind, data))
    }
    fn text(key: HKEY, name: &str) -> String {
        read(key, name)
            .filter(|(k, b)| [1, 2].contains(k) && b.len() % 2 == 0)
            .and_then(|(_, b)| {
                String::from_utf16(
                    &b.chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect::<Vec<_>>(),
                )
                .ok()
            })
            .unwrap_or_default()
            .trim_end_matches('\0')
            .into()
    }
    fn number(key: HKEY, name: &str) -> u32 {
        read(key, name)
            .filter(|(k, b)| *k == 4 && b.len() == 4)
            .map(|(_, b)| u32::from_le_bytes(b.try_into().unwrap()))
            .unwrap_or(0)
    }
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    for (hive, label) in [(HKEY_CURRENT_USER, "HKCU"), (HKEY_LOCAL_MACHINE, "HKLM")] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let path = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
            let mut root = null_mut();
            let code =
                unsafe { RegOpenKeyExW(hive, wide(path).as_ptr(), 0, KEY_READ | view, &mut root) };
            if code == ERROR_FILE_NOT_FOUND {
                continue;
            }
            if code != ERROR_SUCCESS {
                warnings.push(format!("{label}: {code}"));
                continue;
            }
            let root = Key(root);
            for i in 0..20000 {
                let mut name = vec![0u16; 16384];
                let mut n = name.len() as u32;
                let code = unsafe {
                    RegEnumKeyExW(
                        root.0,
                        i,
                        name.as_mut_ptr(),
                        &mut n,
                        null(),
                        null_mut(),
                        null_mut(),
                        null_mut(),
                    )
                };
                if code == ERROR_NO_MORE_ITEMS {
                    break;
                }
                if code != ERROR_SUCCESS {
                    warnings.push(format!("{label} entry {i}: {code}"));
                    continue;
                }
                let name = String::from_utf16(&name[..n as usize])?;
                let mut child = null_mut();
                if unsafe {
                    RegOpenKeyExW(root.0, wide(&name).as_ptr(), 0, KEY_READ | view, &mut child)
                } != 0
                {
                    continue;
                }
                let child = Key(child);
                let display = text(child.0, "DisplayName");
                if display.is_empty() || number(child.0, "SystemComponent") == 1 {
                    continue;
                }
                let command = text(child.0, "UninstallString");
                let product = if number(child.0, "WindowsInstaller") == 1 && guid(&name) {
                    name.clone()
                } else {
                    String::new()
                };
                let removable = number(child.0, "NoRemove") == 0
                    && (!product.is_empty() || command_parts(&command).is_some());
                items.push(App {
                    id: format!("{label}/{view}/{name}"),
                    name: display,
                    publisher: text(child.0, "Publisher"),
                    version: text(child.0, "DisplayVersion"),
                    size_kb: number(child.0, "EstimatedSize") as u64,
                    command,
                    product,
                    removable,
                });
            }
        }
    }
    // HKCU redirected views may resolve to the same physical key.
    items.dedup_by(|a, b| a.name == b.name && a.command == b.command && a.product == b.product);
    Ok((items, warnings))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uninstaller_commands_never_use_a_shell_or_ambiguous_paths() {
        assert_eq!(
            command_parts(r#""C:\Program Files\App\uninstall.exe" /uninstall"#),
            Some((
                r"C:\Program Files\App\uninstall.exe".into(),
                "/uninstall".into()
            ))
        );
        for s in [
            r"C:\Program Files\App\uninstall.exe",
            r"cmd.exe /c erase",
            r"C:\Windows\System32\powershell.exe anything",
            r"C:\Windows\System32\msiexec.exe /x bad",
            r#""C:\app.exe"bad"#,
        ] {
            assert!(command_parts(s).is_none());
        }
    }
    #[test]
    fn msi_product_codes_are_strict() {
        assert!(guid("{12345678-1234-1234-1234-123456789ABC}"));
        for s in [
            "/x anything",
            "{123}",
            "{12345678-1234-1234-1234-123456789ABZ}",
        ] {
            assert!(!guid(s));
        }
    }
}
