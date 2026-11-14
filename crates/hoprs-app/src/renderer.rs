//! Picks the renderer before iced starts.
//!
//! iced tries wgpu first and falls back to tiny-skia only when wgpu finds no adapter at
//! all. On a machine without a GPU, Mesa still offers a software adapter (llvmpipe), so
//! wgpu would run on the CPU: slower than tiny-skia and several times its memory (about
//! 165 MB against 32 MB at start-up, far over the spec §9 idle target of 80 MB).
//!
//! So when `ICED_BACKEND` is unset, hoprs looks at the kernel's GPU render nodes
//! (`/sys/class/drm/renderD*`, Linux only). If there is none, or only software or
//! display-only drivers, it sets `ICED_BACKEND=tiny-skia`. Any other driver, or a `/sys`
//! it cannot read, leaves the choice to iced. Asking wgpu instead would mean loading the
//! GPU drivers twice (about 70 ms); this costs a directory listing.

/// The variable iced reads to choose its renderer.
pub const ENV: &str = "ICED_BACKEND";
/// iced's software renderer.
pub const SOFTWARE: &str = "tiny-skia";

/// Kernel DRM drivers that offer no GPU acceleration: Mesa would render on the CPU
/// through them. Virtual GPUs with 3D (`virtio_gpu`, `vmwgfx`) are not in the list.
const SOFTWARE_DRIVERS: &[&str] = &[
    "vgem",        // virtual GEM provider, software only
    "vkms",        // virtual kernel mode setting, for tests
    "simpledrm",   // firmware framebuffer
    "udl",         // DisplayLink USB, display only
    "evdi",        // DisplayLink (out-of-tree), display only
    "bochs-drm",   // QEMU standard VGA, display only
    "cirrus-qemu", // QEMU Cirrus VGA, display only
    "cirrus",
    "qxl",        // SPICE display, no 3D
    "vboxvideo",  // VirtualBox, no 3D
    "hyperv_drm", // Hyper-V synthetic video, no 3D
];

/// The renderer to force, if any, given the drivers behind the render nodes.
///
/// - `ICED_BACKEND` already set (not empty): `None`, since the user's choice wins.
/// - No render node, or only [`SOFTWARE_DRIVERS`]: [`SOFTWARE`].
/// - Any other driver: `None`, and iced picks wgpu.
pub fn choose_backend(drivers: &[&str], env: Option<&str>) -> Option<&'static str> {
    if env.is_some_and(|value| !value.is_empty()) {
        return None;
    }
    if drivers
        .iter()
        .all(|driver| SOFTWARE_DRIVERS.contains(driver))
    {
        Some(SOFTWARE)
    } else {
        None
    }
}

/// What [`pick`] decided, as one line for the technical log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice(pub String);

/// Sets `ICED_BACKEND=tiny-skia` when the machine has no GPU (see the module docs).
///
/// It may change the environment, so it must run on the main thread before any other
/// thread exists: call it first thing in `run`.
pub fn pick() -> Choice {
    let env = std::env::var(ENV).ok();
    if let Some(value) = env.as_deref().filter(|value| !value.is_empty()) {
        return Choice(format!("renderer: {value} (set by {ENV})"));
    }
    let Some(drivers) = render_drivers() else {
        return Choice("renderer: iced's default (no render node information)".to_owned());
    };
    let drivers: Vec<&str> = drivers.iter().map(String::as_str).collect();
    match choose_backend(&drivers, env.as_deref()) {
        Some(backend) => {
            // SAFETY: called first thing in `run`, on the main thread, before hoprs (or
            // anything it links) starts a thread; nothing reads the environment
            // concurrently.
            //
            // Child processes inherit it: the remote-edit editor (`backend/edit.rs`) and
            // the folder opener (`effects.rs`). That is acceptable: only iced programs
            // read `ICED_BACKEND`, and one started on this machine would make the same
            // choice (no GPU render node, so tiny-skia); everything else ignores it. A
            // value the user set is inherited the same way.
            unsafe { std::env::set_var(ENV, backend) };
            if drivers.is_empty() {
                Choice(format!("renderer: {backend} (no GPU render node)"))
            } else {
                Choice(format!(
                    "renderer: {backend} (only software render nodes: {})",
                    drivers.join(", ")
                ))
            }
        }
        None => Choice(format!(
            "renderer: iced's default, wgpu (GPU render node: {})",
            drivers.join(", ")
        )),
    }
}

/// The driver behind each render node (`/sys/class/drm/renderD*/device/driver`, the
/// symlink's basename); `None` if `/sys/class/drm` cannot be read. A node whose driver
/// link cannot be read counts as an unknown driver, so as a GPU.
#[cfg(target_os = "linux")]
fn render_drivers() -> Option<Vec<String>> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    let mut drivers: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
        .map(|entry| {
            std::fs::read_link(entry.path().join("device/driver"))
                .ok()
                .and_then(|link| {
                    link.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "unknown".to_owned())
        })
        .collect();
    drivers.sort();
    drivers.dedup();
    Some(drivers)
}

/// Elsewhere there is no render node information: iced decides.
#[cfg(not(target_os = "linux"))]
fn render_drivers() -> Option<Vec<String>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_env_value_wins() {
        assert_eq!(choose_backend(&[], Some("wgpu")), None);
        assert_eq!(choose_backend(&["vgem"], Some("wgpu")), None);
        assert_eq!(choose_backend(&["nvidia"], Some("tiny-skia")), None);
    }

    #[test]
    fn no_render_node_gives_tiny_skia() {
        assert_eq!(choose_backend(&[], None), Some(SOFTWARE));
        // Empty counts as unset, as it does for iced.
        assert_eq!(choose_backend(&[], Some("")), Some(SOFTWARE));
    }

    #[test]
    fn only_software_drivers_give_tiny_skia() {
        assert_eq!(choose_backend(&["vgem"], None), Some(SOFTWARE));
        assert_eq!(choose_backend(&["vkms"], None), Some(SOFTWARE));
        assert_eq!(choose_backend(&["vgem", "vkms"], None), Some(SOFTWARE));
    }

    #[test]
    fn any_real_driver_gives_none() {
        for driver in [
            "i915",
            "xe",
            "amdgpu",
            "radeon",
            "nouveau",
            "nvidia",
            "virtio_gpu",
            "vmwgfx",
            "unknown",
        ] {
            assert_eq!(choose_backend(&[driver], None), None, "{driver}");
            assert_eq!(choose_backend(&["vgem", driver], None), None, "{driver}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn render_drivers_reads_sysfs_without_panicking() {
        // Whatever the machine has (CI runners have no render node): just a list.
        if let Some(drivers) = render_drivers() {
            assert!(drivers.iter().all(|driver| !driver.is_empty()));
        }
    }
}
