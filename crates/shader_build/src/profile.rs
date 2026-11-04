use std::{ffi::OsStr, path::Path};

// Cargo 的两种 OUT_DIR 布局；只解析 build 下的包槽，不从任意祖先名称猜测 profile。
pub(crate) fn cargo_profile<'a>(out: &'a Path, package: &str) -> Result<&'a OsStr, &'static str> {
    let invalid = "无法识别 Cargo OUT_DIR 布局，不能确定 shader profile";
    if out.file_name() != Some(OsStr::new("out")) {
        return Err(invalid);
    }
    let slot = out.parent().ok_or(invalid)?;
    let parent = slot.parent().ok_or(invalid)?;
    let build = if parent.file_name() == Some(OsStr::new("build"))
        && slot
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| {
                name.strip_prefix(package)
                    .is_some_and(|suffix| suffix.starts_with('-') && suffix.len() > 1)
            }) {
        parent
    } else if parent.file_name() == Some(OsStr::new(package))
        && parent.parent().and_then(Path::file_name) == Some(OsStr::new("build"))
    {
        parent.parent().ok_or(invalid)?
    } else {
        return Err(invalid);
    };
    build.parent().and_then(Path::file_name).ok_or(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_uses_the_package_build_slot() {
        for path in [
            "/tmp/release-lto/build/shader_build-123/out",
            "/tmp/release-lto/build/shader_build/123/out",
            "/tmp/wasm32-unknown-unknown/release-lto/build/shader_build/123/out",
        ] {
            assert_eq!(
                cargo_profile(Path::new(path), "shader_build"),
                Ok(OsStr::new("release-lto"))
            );
        }
    }

    #[test]
    fn ancestor_names_do_not_override_the_actual_profile() {
        let out = Path::new("/release-lto/custom-build/debug/build/shader_build/123/out");
        assert_eq!(cargo_profile(out, "shader_build"), Ok(OsStr::new("debug")));
    }

    #[test]
    fn unknown_layout_does_not_guess_a_profile() {
        assert!(
            cargo_profile(
                Path::new("/tmp/release-lto/shader_build/123/out"),
                "shader_build"
            )
            .is_err()
        );
    }
}
