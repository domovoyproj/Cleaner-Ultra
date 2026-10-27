//! Native Windows Storage Management ReTrim. Only the filesystem/provider chooses
//! free ranges; never issue raw DSM TRIM against a live mounted volume bitmap.
use anyhow::{Result, bail};
pub fn validate_drive(drive: &str) -> Result<String> {
    if drive.len() != 2 || !drive.as_bytes()[0].is_ascii_alphabetic() || drive.as_bytes()[1] != b':'
    {
        bail!("Выберите локальный диск");
    }
    Ok(drive.to_ascii_uppercase())
}
#[cfg(windows)]
pub fn retrim(drive: &str) -> Result<()> {
    use anyhow::Context;
    use windows::{
        Win32::System::{
            Com::*,
            Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE},
            Variant::VARIANT,
            Wmi::*,
        },
        core::{BSTR, w},
    };
    let drive = validate_drive(drive)?;
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let empty = BSTR::new();
        let service = locator.ConnectServer(
            &BSTR::from(r"ROOT\Microsoft\Windows\Storage"),
            &empty,
            &empty,
            &empty,
            0,
            &empty,
            None,
        )?;
        CoSetProxyBlanket(
            &service,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            None,
            RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        let query = BSTR::from(format!(
            "SELECT * FROM MSFT_Volume WHERE DriveLetter = '{}'",
            drive.as_bytes()[0] as char
        ));
        let objects = service.ExecQuery(
            &BSTR::from("WQL"),
            &query,
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )?;
        let mut rows = [None];
        let mut count = 0;
        objects.Next(30000, &mut rows, &mut count).ok()?;
        if count != 1 {
            bail!("Том {drive} не найден Windows Storage API");
        }
        let volume = rows[0].take().context("Не получен том")?;
        let mut path = VARIANT::default();
        volume.Get(w!("__PATH"), 0, &mut path, None, None)?;
        let path = BSTR::try_from(&path)?;
        let mut class = None;
        service.GetObject(
            &BSTR::from("MSFT_Volume"),
            WBEM_GENERIC_FLAG_TYPE(0),
            None,
            Some(&mut class),
            None,
        )?;
        let mut signature = None;
        class.context("Нет MSFT_Volume")?.GetMethod(
            w!("Optimize"),
            0,
            &mut signature,
            std::ptr::null_mut(),
        )?;
        let input = signature
            .context("Нет параметров Optimize")?
            .SpawnInstance(0)?;
        for (key, enabled) in [
            (w!("ReTrim"), true),
            (w!("Analyze"), false),
            (w!("Defrag"), false),
            (w!("SlabConsolidate"), false),
            (w!("TierOptimize"), false),
        ] {
            input.Put(key, 0, &VARIANT::from(enabled), 0)?;
        }
        let mut output = None;
        service.ExecMethod(
            &path,
            &BSTR::from("Optimize"),
            WBEM_GENERIC_FLAG_TYPE(0),
            None,
            &input,
            Some(&mut output),
            None,
        )?;
        let output = output.context("Windows не вернула результат TRIM")?;
        let mut result = VARIANT::default();
        output.Get(w!("ReturnValue"), 0, &mut result, None, None)?;
        let result = u32::try_from(&result)?;
        if result != 0 {
            let mut detail = VARIANT::default();
            let _ = output.Get(w!("ExtendedStatus"), 0, &mut detail, None, None);
            let detail = BSTR::try_from(&detail)
                .map(|s| s.to_string())
                .unwrap_or_default();
            bail!("Windows ReTrim завершился с кодом {result}: {detail}");
        }
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn retrim(_: &str) -> Result<()> {
    bail!("TRIM доступен только на Windows")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retrim_rejects_non_volume_inputs() {
        for s in [
            "C: & erase",
            "C:\\",
            "\\\\server",
            "-x",
            "C:\\file",
            "",
            ":C",
        ] {
            assert!(validate_drive(s).is_err());
        }
        assert_eq!(validate_drive("c:").unwrap(), "C:");
    }
}
#[cfg(all(test, windows))]
mod native_qa {
    #[test]
    #[ignore = "Requires admin and explicit user authorization to ReTrim free volume blocks"]
    fn native_retrim_own_data_preserved() {
        assert!(crate::space::is_administrator());
        let base =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".cleaner_ultra/native-qa");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("trim-")
            .tempdir_in(&base)
            .unwrap();
        let path = dir.path().join("own-allocated-data.bin");
        let data = vec![0x5au8; 8 * 1024 * 1024];
        std::fs::write(&path, &data).unwrap();
        let before = crate::analysis::fingerprint(&path).unwrap();
        let path_string = std::fs::canonicalize(&path)
            .unwrap()
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        super::retrim(&path_string[..2]).unwrap();
        assert_eq!(crate::analysis::fingerprint(&path).unwrap(), before);
        assert_eq!(std::fs::read(&path).unwrap(), data);
        println!("Windows Storage API ReTrim completed; own allocated file SHA-256 preserved");
    }
}
