//! The Android build (`m2-alpha-android`, a phone or tablet with a
//! keyboard and mouse, as in Samsung DeX): the activity's entry point, the
//! app's folder, and mouse capture.
//!
//! The game is read from the app's own folder on the phone's storage
//! (`Android/data/<package>/files`), where the player copies their install's
//! `data` folder. Settings, saves, the catalogue cache, the log
//! (`m2-alpha.log`) and crash reports are kept there too. Arguments the
//! desktop takes on its command line can go in `m2-alpha-args.txt` there.
//! Movies are skipped: they need `ffmpeg`, which the phone doesn't have.

use std::path::Path;

use jni::JavaVM;
use jni::objects::JObject;
use winit::event_loop::EventLoop;
use winit::platform::android::EventLoopBuilderExtAndroid;
use winit::platform::android::activity::AndroidApp;

const ARGS_FILE: &str = "m2-alpha-args.txt";
const LOG_FILE: &str = "m2-alpha.log";

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    let home = app
        .external_data_path()
        .or_else(|| app.internal_data_path());
    if let Some(home) = &home {
        let _ = std::fs::create_dir_all(home);
        let _ = std::env::set_current_dir(home);
        log_to(&home.join(LOG_FILE));
    }
    super::keep_crash_reports();
    let mut args: Vec<String> = std::fs::read_to_string(ARGS_FILE)
        .map(|text| text.split_whitespace().map(String::from).collect())
        .unwrap_or_default();
    if let Some(home) = &home {
        eprintln!("app folder: {}", home.display());
        if !args.iter().any(|a| a == "--install") {
            match m2_engine::Install::find(home) {
                Ok(_) => args.extend(["--install".into(), home.display().to_string()]),
                Err(e) => eprintln!("{e}: copy the game's data folder here"),
            }
        }
    }
    eprintln!("arguments: {args:?}");
    if let Err(e) = super::run(&args, || EventLoop::builder().with_android_app(app).build()) {
        eprintln!("error: {e:#}");
    }
}

/// Sends standard error (where the game reports) to `path`: an app has no
/// console.
fn log_to(path: &Path) {
    use std::os::fd::AsRawFd as _;
    if let Ok(file) = std::fs::File::create(path) {
        // SAFETY: both descriptors are open; dup2 leaves `file`'s own to be
        // closed when it drops.
        unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) };
    }
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
    let ctx = ndk_context::android_context();
    // SAFETY: the context holds the process's JavaVM and the activity, both
    // alive while the app runs.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // SAFETY: as above; the activity is a global reference not freed here.
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let result = (|| {
        let window = env
            .call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?
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
    })();
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    result
}
