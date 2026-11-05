//! ZIP 打包（详细设计 11.4）：图片直接存储、其他压缩；条目名去掉危险路径、同名的加序号；
//! 先写临时文件再改名。

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::naming::sanitize_file_name;
use crate::{IoError, Result};

/// 已经压缩过的格式，再压缩省不了空间，直接存储更快。
const STORED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "zip", "xlsx"];

/// 条目内容来自哪里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipSource {
    File(PathBuf),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    /// 包里的路径，用 `/` 分隔，如 `images/T恤_1.png`
    pub name: String,
    pub source: ZipSource,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipReport {
    pub files: usize,
    /// ZIP 文件大小
    pub bytes: u64,
    /// 改过名的条目：(原名, 实际名)
    pub renamed: Vec<(String, String)>,
}

/// 写 ZIP 文件到 `target`。
pub fn write_zip(target: &Path, entries: &[ZipEntry]) -> Result<ZipReport> {
    let mut temp_name = target.as_os_str().to_owned();
    temp_name.push(".tmp");
    let temp = PathBuf::from(temp_name);
    let result = write_to(&temp, entries).and_then(|report| {
        std::fs::rename(&temp, target).map_err(|error| IoError::write(target, error))?;
        let bytes = std::fs::metadata(target)
            .map_err(|error| IoError::read(target, error))?
            .len();
        Ok(ZipReport { bytes, ..report })
    });
    if result.is_err()
        && let Err(error) = std::fs::remove_file(&temp)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(IoError::write(&temp, error));
    }
    result
}

fn write_to(path: &Path, entries: &[ZipEntry]) -> Result<ZipReport> {
    let file = std::fs::File::create(path).map_err(|error| IoError::write(path, error))?;
    let mut zip = ZipWriter::new(file);
    let mut used = HashSet::new();
    let mut report = ZipReport::default();
    for entry in entries {
        let name = unique_name(&entry_name(&entry.name), &mut used);
        if name != entry.name {
            report.renamed.push((entry.name.clone(), name.clone()));
        }
        let method = if is_stored(&name) {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        let options = SimpleFileOptions::default().compression_method(method);
        zip.start_file(name.as_str(), options)
            .map_err(|error| IoError::Generate(error.to_string()))?;
        match &entry.source {
            ZipSource::Bytes(bytes) => zip
                .write_all(bytes)
                .map_err(|error| IoError::write(path, error))?,
            ZipSource::File(source) => {
                let mut file =
                    std::fs::File::open(source).map_err(|error| IoError::read(source, error))?;
                std::io::copy(&mut file, &mut zip).map_err(|error| IoError::read(source, error))?;
            }
        }
        report.files += 1;
    }
    let file = zip
        .finish()
        .map_err(|error| IoError::Generate(error.to_string()))?;
    file.sync_all()
        .map_err(|error| IoError::write(path, error))?;
    Ok(report)
}

/// 去掉 `..`、开头的 `/`、盘符和非法字符，每一级都按文件名规则清理。
fn entry_name(name: &str) -> String {
    let parts: Vec<String> = name
        .replace('\\', "/")
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(sanitize_file_name)
        .collect();
    if parts.is_empty() {
        sanitize_file_name("")
    } else {
        parts.join("/")
    }
}

/// 同名（不区分大小写，Windows 解压时会冲突）的加 (2)、(3)。
fn unique_name(name: &str, used: &mut HashSet<String>) -> String {
    if used.insert(name.to_lowercase()) {
        return name.to_owned();
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > name.rfind('/').map_or(0, |slash| slash + 1) => name.split_at(dot),
        _ => (name, ""),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){extension}"))
        .find(|candidate| used.insert(candidate.to_lowercase()))
        .unwrap_or_else(|| name.to_owned())
}

fn is_stored(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| STORED_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_are_made_safe() {
        assert_eq!(entry_name("../../etc/passwd"), "etc/passwd");
        assert_eq!(entry_name("\\images\\a:b.png"), "images/a_b.png");
        assert_eq!(entry_name("/"), "未命名");
    }

    #[test]
    fn duplicates_get_numbers() {
        let mut used = HashSet::new();
        assert_eq!(unique_name("images/a.png", &mut used), "images/a.png");
        assert_eq!(unique_name("images/A.png", &mut used), "images/A (2).png");
        assert_eq!(unique_name("images/a.png", &mut used), "images/a (3).png");
        assert_eq!(unique_name("v1.0/readme", &mut used), "v1.0/readme");
        assert_eq!(unique_name("v1.0/readme", &mut used), "v1.0/readme (2)");
    }

    #[test]
    fn images_are_stored() {
        assert!(is_stored("a/B.PNG"));
        assert!(!is_stored("list.csv"));
        assert!(!is_stored("noext"));
    }
}
