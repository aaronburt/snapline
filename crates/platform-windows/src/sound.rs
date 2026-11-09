use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_FILENAME};

pub const SOUNDS_DIR: &str = "assets/sounds";
pub const SHUTTER_FILE: &str = "shutter.wav";
pub const DONE_FILE: &str = "done.wav";
pub const FALLBACK_ALIAS: &str = "SystemAsterisk";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundId {
    Shutter,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Playback {
    File(PathBuf),
    Alias(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundPaths {
    root: PathBuf,
}

impl SoundId {
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Shutter => SHUTTER_FILE,
            Self::Done => DONE_FILE,
        }
    }

    pub const fn fallback_alias(self) -> &'static str {
        FALLBACK_ALIAS
    }
}

impl SoundPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn beside_exe() -> Self {
        let root = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join(SOUNDS_DIR)))
            .unwrap_or_else(|| PathBuf::from(SOUNDS_DIR));
        Self::new(root)
    }

    pub fn file(&self, sound: SoundId) -> PathBuf {
        self.root.join(sound.file_name())
    }

    pub fn selection(&self, sound: SoundId) -> Playback {
        let file = self.file(sound);
        if file.is_file() {
            Playback::File(file)
        } else {
            Playback::Alias(sound.fallback_alias())
        }
    }
}

pub fn play(sound: SoundId) -> Playback {
    play_in(&SoundPaths::beside_exe(), sound)
}

pub fn play_in(paths: &SoundPaths, sound: SoundId) -> Playback {
    let playback = paths.selection(sound);
    match &playback {
        Playback::File(path) => {
            let path: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: path is a live null-terminated UTF-16 string that outlives the call.
            let _ = unsafe { PlaySoundW(PCWSTR(path.as_ptr()), None, SND_ASYNC | SND_FILENAME) };
        }
        Playback::Alias(alias) => {
            let alias: Vec<u16> = alias.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: alias is a live null-terminated UTF-16 string that outlives the call.
            let _ = unsafe { PlaySoundW(PCWSTR(alias.as_ptr()), None, SND_ASYNC | SND_ALIAS) };
        }
    }
    playback
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn sound_ids_name_the_documented_wav_files() {
        assert_eq!(SOUNDS_DIR, "assets/sounds");
        assert_eq!(SoundId::Shutter.file_name(), SHUTTER_FILE);
        assert_eq!(SoundId::Done.file_name(), DONE_FILE);
        assert_eq!(SHUTTER_FILE, "shutter.wav");
        assert_eq!(DONE_FILE, "done.wav");
    }

    #[test]
    fn sound_paths_join_the_asset_directory() {
        let paths = SoundPaths::new(SOUNDS_DIR);
        assert_eq!(
            paths.file(SoundId::Shutter),
            Path::new(SOUNDS_DIR).join("shutter.wav")
        );
        assert_eq!(
            paths.file(SoundId::Done),
            Path::new(SOUNDS_DIR).join("done.wav")
        );
    }

    #[test]
    fn missing_file_selects_the_alias_fallback() {
        let Ok(directory) = tempfile::tempdir() else {
            return;
        };
        let paths = SoundPaths::new(directory.path());
        assert_eq!(
            paths.selection(SoundId::Shutter),
            Playback::Alias("SystemAsterisk")
        );
        assert_eq!(
            paths.selection(SoundId::Done),
            Playback::Alias("SystemAsterisk")
        );
        assert_eq!(FALLBACK_ALIAS, "SystemAsterisk");
    }

    #[test]
    fn present_file_is_preferred() {
        let Ok(directory) = tempfile::tempdir() else {
            return;
        };
        let paths = SoundPaths::new(directory.path());
        let shutter = paths.file(SoundId::Shutter);
        let done = paths.file(SoundId::Done);
        if std::fs::write(&shutter, [0_u8; 8]).is_err() || std::fs::write(&done, [0_u8; 8]).is_err()
        {
            return;
        }
        assert_eq!(paths.selection(SoundId::Shutter), Playback::File(shutter));
        assert_eq!(paths.selection(SoundId::Done), Playback::File(done));
    }

    #[test]
    #[ignore = "desktop"]
    fn missing_sound_plays_the_system_alias_without_panicking(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = SoundPaths::new(directory.path());
        assert_eq!(
            play_in(&paths, SoundId::Shutter),
            Playback::Alias(FALLBACK_ALIAS)
        );
        assert_eq!(
            play_in(&paths, SoundId::Done),
            Playback::Alias(FALLBACK_ALIAS)
        );
        Ok(())
    }
}
