// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! File classes by extension (F0-diseno.md §1.5).

/// File class, stored as `u8` in the `class` column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Class {
    /// Anything else, including directories, symlinks and special files.
    Other = 0,
    /// Source code and configuration.
    Code = 1,
    /// Images.
    Image = 2,
    /// Video.
    Video = 3,
    /// Audio.
    Audio = 4,
    /// Documents.
    Document = 5,
    /// Archives and compressed files.
    Archive = 6,
    /// Binaries, libraries and disk images.
    Binary = 7,
}

impl Class {
    /// Number of classes.
    pub const COUNT: usize = 8;

    /// Converts a stored byte back into a class. Unknown values map to
    /// [`Class::Other`].
    pub const fn from_u8(v: u8) -> Class {
        match v {
            1 => Class::Code,
            2 => Class::Image,
            3 => Class::Video,
            4 => Class::Audio,
            5 => Class::Document,
            6 => Class::Archive,
            7 => Class::Binary,
            _ => Class::Other,
        }
    }

    /// Classifies a lowercase extension (without the dot).
    pub fn from_extension(ext: &[u8]) -> Class {
        match ext {
            b"rs" | b"c" | b"h" | b"cpp" | b"hpp" | b"py" | b"js" | b"ts" | b"tsx" | b"jsx"
            | b"go" | b"java" | b"kt" | b"rb" | b"php" | b"sh" | b"lua" | b"zig" | b"toml"
            | b"yaml" | b"yml" | b"json" | b"html" | b"css" | b"scss" | b"sql" => Class::Code,
            b"jpg" | b"jpeg" | b"png" | b"gif" | b"webp" | b"avif" | b"heic" | b"bmp" | b"tiff"
            | b"svg" | b"ico" | b"dng" | b"cr2" | b"nef" | b"arw" | b"psd" | b"xcf" | b"kra" => {
                Class::Image
            }
            b"mp4" | b"mkv" | b"webm" | b"mov" | b"avi" | b"m4v" | b"m2ts" => Class::Video,
            b"mp3" | b"flac" | b"ogg" | b"opus" | b"wav" | b"m4a" | b"aac" => Class::Audio,
            b"pdf" | b"odt" | b"ods" | b"odp" | b"doc" | b"docx" | b"xls" | b"xlsx" | b"ppt"
            | b"pptx" | b"txt" | b"md" | b"epub" | b"rtf" | b"csv" | b"tex" => Class::Document,
            b"zip" | b"tar" | b"gz" | b"tgz" | b"xz" | b"zst" | b"bz2" | b"7z" | b"rar" => {
                Class::Archive
            }
            b"so" | b"a" | b"o" | b"exe" | b"dll" | b"bin" | b"deb" | b"rpm" | b"appimage"
            | b"iso" | b"img" | b"qcow2" | b"vdi" | b"vmdk" => Class::Binary,
            _ => Class::Other,
        }
    }
}

/// Longest extension that is interned; longer suffixes count as "no
/// extension" so that random dotted names do not bloat the table.
pub const MAX_EXTENSION_LEN: usize = 16;

/// Returns the extension of `name` (bytes after the last `.`), if any.
///
/// A leading dot (`.bashrc`) or a trailing dot (`file.`) is not an
/// extension.
pub fn extension(name: &[u8]) -> Option<&[u8]> {
    let dot = name.iter().rposition(|&b| b == b'.')?;
    if dot == 0 || dot + 1 == name.len() || name.len() - dot - 1 > MAX_EXTENSION_LEN {
        return None;
    }
    Some(&name[dot + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(extension(b"a.tar.gz"), Some(&b"gz"[..]));
        assert_eq!(extension(b".bashrc"), None);
        assert_eq!(extension(b"file."), None);
        assert_eq!(extension(b"noext"), None);
        assert_eq!(extension(b"x.abcdefghijklmnopq"), None);
    }

    #[test]
    fn classes() {
        assert_eq!(Class::from_extension(b"ts"), Class::Code);
        assert_eq!(Class::from_extension(b"m2ts"), Class::Video);
        assert_eq!(Class::from_extension(b"appimage"), Class::Binary);
        assert_eq!(Class::from_extension(b"unknown"), Class::Other);
        for v in 0..=255u8 {
            let c = Class::from_u8(v);
            if v < 8 {
                assert_eq!(c as u8, v);
            } else {
                assert_eq!(c, Class::Other);
            }
        }
    }
}
