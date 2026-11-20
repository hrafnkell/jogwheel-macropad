//! `jogwheel --install`: app menu entry + desktop shortcut with the app icon.
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
pub fn install() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let exe = std::env::current_exe()?;
    let home = PathBuf::from(std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("no HOME"))?);
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    for (size, png) in [
        (32, &include_bytes!("../assets/icon-32.png")[..]),
        (64, &include_bytes!("../assets/icon-64.png")[..]),
        (128, &include_bytes!("../assets/icon-128.png")[..]),
        (256, &include_bytes!("../assets/icon-256.png")[..]),
        (512, &include_bytes!("../assets/icon-512.png")[..]),
    ] {
        let dir = data.join(format!("icons/hicolor/{size}x{size}/apps"));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("jogwheel.png"), png)?;
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Jogwheel\nGenericName=Macropad configurator\n\
         Comment=Configure the jogwheel macropad and switch its layers to match the focused app\n\
         Exec=\"{}\"\nIcon=jogwheel\nTerminal=false\nCategories=Utility;Settings;HardwareSettings;\n\
         Keywords=macropad;keyboard;jog;shuttle;layers;\nStartupWMClass=jogwheel\n",
        exe.display()
    );
    let apps = data.join("applications");
    std::fs::create_dir_all(&apps)?;
    let menu_file = apps.join("jogwheel.desktop");
    std::fs::write(&menu_file, &entry)?;
    println!("App menu entry: {}", menu_file.display());

    // Desktop shortcut (executable + "trusted" so file managers launch it)
    let desktop = std::process::Command::new("xdg-user-dir")
        .arg("DESKTOP")
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|p| p.is_dir() && p != &home)
        .unwrap_or_else(|| home.join("Desktop"));
    if desktop.is_dir() {
        let f = desktop.join("jogwheel.desktop");
        std::fs::write(&f, &entry)?;
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755))?;
        let _ = std::process::Command::new("gio").args(["set", &f.to_string_lossy(), "metadata::trusted", "true"]).output();
        println!("Desktop shortcut: {}", f.display());
    }
    let _ = std::process::Command::new("update-desktop-database").arg(&apps).output();
    let _ = std::process::Command::new("gtk-update-icon-cache").args(["-q", "-t"]).arg(data.join("icons/hicolor")).output();
    Ok(())
}

#[cfg(windows)]
pub fn install() -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    let exe = std::env::current_exe()?;
    let exe = exe.display().to_string().replace('\'', "''");
    // Shortcuts via the WScript.Shell COM object; the icon comes from the exe.
    let script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         foreach ($dir in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) {{ \
           $s = $ws.CreateShortcut((Join-Path $dir 'Jogwheel.lnk')); $s.TargetPath = '{exe}'; \
           $s.Description = 'Jogwheel macropad configurator'; $s.Save(); Write-Output (Join-Path $dir 'Jogwheel.lnk') }}"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .output()?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    print!("Shortcuts:\n{}", String::from_utf8_lossy(&out.stdout));
    Ok(())
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn install() -> anyhow::Result<()> {
    anyhow::bail!("--install isn't supported on this OS")
}
