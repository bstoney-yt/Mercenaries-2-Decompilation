//! The Android build (`m2-alpha-android`, a phone or tablet with a
//! keyboard and mouse, as in Samsung DeX): the activity's entry point, the
//! game's folder, and mouse capture.
//!
//! The game is read from the `Mercenaries2Alpha` folder at the top of the
//! phone's storage (or the top of the storage, or the downloads), where the player drags a folder holding the APK and
//! their install's `data` folder. Reading it needs "all files access",
//! which the app asks for (by opening its settings page) when it can't read
//! the game there. The app's own folder (`Android/data/<package>/files`)
//! works too, without that permission. Settings, saves, the catalogue cache
//! and crash reports are kept beside the game; the log (`m2-alpha.log`) goes
//! to the downloads.
//! Arguments the desktop takes on its command line can go in
//! `m2-alpha-args.txt` there. Movies are skipped: they need `ffmpeg`, which
//! the phone doesn't have.

use std::path::Path;

use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};
use m2_engine::Install;
use winit::event_loop::EventLoop;
use winit::platform::android::EventLoopBuilderExtAndroid;
use winit::platform::android::activity::AndroidApp;

const ARGS_FILE: &str = "m2-alpha-args.txt";
const LOG_FILE: &str = "m2-alpha.log";
/// The top of the phone's shared storage.
const STORAGE: &str = "/storage/emulated/0";
/// The folder the player drags onto the phone (its name), and where else
/// the game may have landed: the top of the storage, or the downloads.
const SHARED: [&str; 4] = [
    "Mercenaries2Alpha",
    "",
    "Download/Mercenaries2Alpha",
    "Download",
];

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    let own = app
        .external_data_path()
        .or_else(|| app.internal_data_path());
    // The log goes to the downloads, which the player can open without
    // all files access (an app may add files there), else beside the game.
    let downloads = Path::new(STORAGE).join("Download").join(LOG_FILE);
    let logged = log_to(&downloads);
    let access = all_files_access();
    let found = SHARED
        .iter()
        .map(|dir| Path::new(STORAGE).join(dir))
        .chain(own.clone())
        .find(|dir| Install::find(dir).is_ok());
    let home = found.clone().or(own);
    if let Some(home) = &home {
        let _ = std::fs::create_dir_all(home);
        let _ = std::env::set_current_dir(home);
        if !logged {
            log_to(&home.join(LOG_FILE));
        }
    }
    super::keep_crash_reports();
    eprintln!("all files access: {access:?}");
    match &found {
        Some(dir) => eprintln!("game found in {}", dir.display()),
        None => {
            eprintln!(
                "the game was not found: put the install's data folder in {STORAGE}/{}",
                SHARED[0]
            );
            if !matches!(access, Ok(true)) {
                eprintln!("asking for all files access; open the game again after allowing it");
                if let Err(e) = with_activity(ask_all_files_access) {
                    eprintln!("could not open the all files access page: {e}");
                }
            }
        }
    }
    let mut args: Vec<String> = std::fs::read_to_string(ARGS_FILE)
        .map(|text| text.split_whitespace().map(String::from).collect())
        .unwrap_or_default();
    if let Some(found) = &found
        && !args.iter().any(|a| a == "--install")
    {
        args.extend(["--install".into(), found.display().to_string()]);
    }
    eprintln!("arguments: {args:?}");
    if let Err(e) = super::run(&args, || EventLoop::builder().with_android_app(app).build()) {
        eprintln!("error: {e:#}");
    }
}

/// Sends standard error (where the game reports) to `path`: an app has no
/// console. False when the file can't be made.
fn log_to(path: &Path) -> bool {
    use std::os::fd::AsRawFd as _;
    let Ok(file) = std::fs::File::create(path) else {
        return false;
    };
    // SAFETY: both descriptors are open; dup2 leaves `file`'s own to be
    // closed when it drops.
    unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) != -1 }
}

/// Captures the mouse (`on`) or lets it go: while captured it moves no
/// cursor and its raw motion turns the camera, as with the desktop's grab
/// (winit has none on Android). Its motion arrives either way, so a failure
/// is only reported.
pub fn capture_pointer(on: bool) {
    if let Err(e) = try_capture(on) {
        eprintln!("pointer capture: {e}");
    }
}

fn try_capture(on: bool) -> jni::errors::Result<()> {
    with_activity(|env, activity| {
        let window = env
            .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let view = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        let method = if on {
            "requestPointerCapture"
        } else {
            "releasePointerCapture"
        };
        env.call_method(&view, method, "()V", &[])?;
        Ok(())
    })
}

/// Whether the app may read any file on the shared storage (Android 11 and
/// later ask for this).
fn all_files_access() -> jni::errors::Result<bool> {
    with_activity(|env, _| {
        env.call_static_method(
            "android/os/Environment",
            "isExternalStorageManager",
            "()Z",
            &[],
        )?
        .z()
    })
}

/// Opens the app's "all files access" settings page.
fn ask_all_files_access(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<()> {
    let package = env
        .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
        .l()?;
    let package: String = env.get_string(&package.into())?.into();
    let uri = env.new_string(format!("package:{package}"))?;
    let uri = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&uri)],
        )?
        .l()?;
    let action = env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
    let intent = env.new_object(
        "android/content/Intent",
        "(Ljava/lang/String;Landroid/net/Uri;)V",
        &[JValue::Object(&action), JValue::Object(&uri)],
    )?;
    env.call_method(
        activity,
        "startActivity",
        "(Landroid/content/Intent;)V",
        &[JValue::Object(&intent)],
    )?;
    Ok(())
}

/// Runs `f` with the JVM attached to this thread and the activity, clearing
/// any Java exception it leaves.
fn with_activity<T>(
    f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>,
) -> jni::errors::Result<T> {
    let ctx = ndk_context::android_context();
    // SAFETY: the context holds the process's JavaVM and the activity, both
    // alive while the app runs.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // SAFETY: as above; the activity is a global reference not freed here.
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let result = f(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    result
}
