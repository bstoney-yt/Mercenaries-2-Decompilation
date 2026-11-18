//! The window's and taskbar's icon: the original game's own, read from
//! the player's install at start (never shipped with this project).
//!
//! The install keeps the icon twice: `mercs2.ico` beside the program, and
//! the program's own icon resource (`Mercenaries2.exe`, group 101). Both
//! hold one 64x64 24-bit image on the install this was checked on. The
//! `.ico` is tried first, then the program, then any other program in the
//! install's folder; the files are only read. Without either the window
//! keeps the system's default icon, and the log says so.
//!
//! Invented: which file is tried first (the two images are the same).

use std::path::Path;

use m2_assets::icon::{self, IconImage};
use winit::window::Icon;

/// The window's (small) and the taskbar's (large) icon.
#[derive(Debug)]
pub struct Icons {
    pub window: Icon,
    pub taskbar: Icon,
}

/// The game's icon, from the install in `root`.
pub fn load(root: &Path) -> Option<Icons> {
    let Some((from, images)) = candidates(root).into_iter().find_map(|path| {
        let bytes = std::fs::read(&path).ok()?;
        let images = if is_ico(&path) {
            icon::ico_file(&bytes)
        } else {
            icon::program_icon(&bytes)
        };
        Some((path, images.ok()?))
    }) else {
        eprintln!("icon: none found in the install; the default one");
        return None;
    };
    let window = to_icon(icon::largest_up_to(&images, 32)?)?;
    let largest = icon::largest_up_to(&images, 256)?;
    let taskbar = to_icon(largest)?;
    eprintln!(
        "icon: from {} ({}x{})",
        from.file_name()
            .map_or_else(Default::default, |n| n.to_string_lossy()),
        largest.width,
        largest.height
    );
    Some(Icons { window, taskbar })
}

fn is_ico(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ico"))
}

/// The files to try, in order: the install's `.ico`, its program, then
/// any other program in its folder.
fn candidates(root: &Path) -> Vec<std::path::PathBuf> {
    let mut found = vec![root.join("mercs2.ico"), root.join("Mercenaries2.exe")];
    let mut others: Vec<_> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) && !found.contains(p)
        })
        .collect();
    others.sort();
    found.append(&mut others);
    found
}

fn to_icon(image: &IconImage) -> Option<Icon> {
    Icon::from_rgba(image.rgba.clone(), image.width, image.height).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tries_the_ico_then_the_program_then_other_programs() {
        let dir = std::env::temp_dir().join(format!("m2-icon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Other.EXE"), b"").unwrap();
        std::fs::write(dir.join("readme.txt"), b"").unwrap();
        let names: Vec<_> = candidates(&dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["mercs2.ico", "Mercenaries2.exe", "Other.EXE"]);
    }

    #[test]
    fn an_install_without_an_icon_gives_none() {
        let dir = std::env::temp_dir().join(format!("m2-no-icon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mercs2.ico"), b"not an icon").unwrap();
        let icons = load(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(icons.is_none());
    }

    /// With `M2_INSTALL` set to an install: its icon is found and read
    /// (`cargo test -p m2-alpha game_icon -- --ignored`).
    #[test]
    #[ignore = "needs the player's install"]
    fn reads_the_installed_icon() {
        let root = std::env::var_os("M2_INSTALL").expect("M2_INSTALL");
        let root = Path::new(&root);
        for path in candidates(root).iter().take(2) {
            let bytes = std::fs::read(path).unwrap();
            let images = if is_ico(path) {
                icon::ico_file(&bytes)
            } else {
                icon::program_icon(&bytes)
            }
            .unwrap();
            let sizes: Vec<_> = images.iter().map(|i| (i.width, i.height)).collect();
            let opaque = images[0].rgba.chunks(4).filter(|p| p[3] == 255).count();
            eprintln!("{}: {sizes:?}, {opaque} opaque pixels", path.display());
            assert!(opaque > 0);
        }
        assert!(load(root).is_some());
    }
}
