//! Windows 薄接缝:PATH 注入写 HKCU 用户 PATH,shim 为 .cmd,Node 为 zip。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::prefix::PathInjection;

/// 把 bin_dir 追加进 HKCU 用户 PATH(幂等,保留原有 REG_EXPAND_SZ 类型)。
pub fn ensure_path(bin_dir: &Path) -> io::Result<Vec<PathInjection>> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu.create_subkey("Environment")?;
    let dir = bin_dir.to_string_lossy().into_owned();

    // 现有值可能不存在;保留 REG_EXPAND_SZ(用户 PATH 常含 %VAR%)
    let raw = env.get_raw_value("Path").ok();
    let existing = match &raw {
        Some(v) => v.to_string(),
        None => String::new(),
    };
    let Some(merged) = super::windows_path_merge(&existing, &dir) else {
        return Ok(Vec::new()); // 已在 PATH,重跑无副作用
    };

    if let Some(v) = raw {
        if v.vtype == REG_EXPAND_SZ {
            // REG_EXPAND_SZ 无现成构造函数:手工编码为带 NUL 结尾的 UTF-16LE
            let bytes: Vec<u8> = merged
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect();
            let value = winreg::RegValue {
                vtype: REG_EXPAND_SZ,
                bytes: bytes.into(),
            };
            env.set_raw_value("Path", &value)?;
        } else {
            env.set_value("Path", &merged)?;
        }
    } else {
        env.set_value("Path", &merged)?;
    }
    // 注:不广播 WM_SETTINGCHANGE(需额外 crate);新开的终端自然生效。
    Ok(vec![PathInjection::WindowsUserPath {
        dir: bin_dir.to_path_buf(),
    }])
}

pub fn write_shim(
    bin_dir: &Path,
    node_bin_dir: &Path,
    npm_bin_dir: &Path,
    bin: &str,
) -> io::Result<PathBuf> {
    fs::create_dir_all(bin_dir)?;
    let path = bin_dir.join(super::shim_file_name(bin));
    fs::write(&path, super::shim_content(node_bin_dir, npm_bin_dir, bin))?;
    Ok(path)
}

pub fn install_self(bin_dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(bin_dir)?;
    let current = std::env::current_exe()?;
    let dest = bin_dir.join(super::exe_name("setup-coder"));
    if current == dest {
        return Ok(dest);
    }
    fs::copy(&current, &dest)?;
    Ok(dest)
}

/// Windows:zip 解压,剥掉顶层目录一层。
pub fn extract_node_archive(archive: &Path, dest_dir: &Path) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("zip 损坏:{e}")))?;
    fs::create_dir_all(dest_dir)?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let stripped: PathBuf = path.components().skip(1).collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        let out = dest_dir.join(stripped);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut w = fs::File::create(&out)?;
            io::copy(&mut entry, &mut w)?;
        }
    }
    Ok(())
}
