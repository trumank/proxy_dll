use proxy_dll::windows_sys::Win32::System::LibraryLoader::LoadLibraryW;

proxy_dll::proxy_dll!([version, winmm, xinput1_3], main);

fn main() {
    load_dlls().ok();
}

fn load_dlls() -> Result<(), std::io::Error> {
    let exe = std::env::current_exe()?;
    let Some(exe_dir) = exe.parent() else {
        return Ok(());
    };
    for entry in std::fs::read_dir(exe_dir.join("dlls"))? {
        let path = entry.unwrap().path();
        if path
            .extension()
            .unwrap_or_default()
            .eq_ignore_ascii_case("dll")
        {
            let path = path.into_os_string();

            use std::os::windows::ffi::OsStrExt;
            let path_encoded: Vec<u16> = path.encode_wide().chain([0]).collect();

            unsafe { LoadLibraryW(path_encoded.as_ptr()) };
        }
    }
    Ok(())
}
