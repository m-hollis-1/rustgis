//! A layout picture's image and the file it is read from: the one reader both commands use, and
//! what they refuse.

use super::*;

/// `image` or `path` given to an element that isn't a picture.
pub(in crate::cmd::layout) const ONLY_PICTURE_FILE: &str = "only pictures take an image or a path";
/// A `path` that isn't text, or is blank.
pub(in crate::cmd::layout) const PICTURE_PATH: &str = "path is the path of a PNG or JPEG file";
/// An `image` that isn't text.
pub(in crate::cmd::layout) const PICTURE_IMAGE: &str = "image is a PNG or JPEG image in base64";
/// A `path` that is an address, which is never tried as a file nor fetched, and never repeated
/// (it could carry a login).
pub(in crate::cmd::layout) const PICTURE_ADDRESS: &str = "a picture is read from a file on this computer, not a web address";
/// Both `image` and `path` given.
pub(in crate::cmd::layout) const PICTURE_BOTH: &str = "give a picture an image or a path, not both";

/// A picture whose header reads but whose image can't be drawn.
pub(in crate::cmd::layout) fn picture_undrawn() -> String {
    format!(
        "the picture can't be drawn: its image is damaged or larger than {} pixels a side",
        rustgis_render::layout::PICTURE_MAX_DECLARED
    )
}

/// A picture read from a file whose path isn't kept as its file.
pub(in crate::cmd::layout) fn picture_unremembered() -> String {
    format!(
        "the picture is read, but its file isn't remembered: a path with control characters or over {} bytes isn't kept",
        rustgis_map::layout::MAX_PICTURE_SOURCE
    )
}

/// Says each of [`rustgis_render::layout::pictures_notices`] of `gaps`, the pictures an export
/// draws as a grey box, with `prefix` before it.
pub(in crate::cmd) fn say_pictures_not_drawn(s: &mut Session, gaps: &[rustgis_render::layout::PictureGap], prefix: &str) {
    for note in rustgis_render::layout::pictures_notices(gaps) {
        s.warn(format!("{prefix}{note}"));
    }
}

/// A picture's image as a command gives it, and the file it was read from.
pub(crate) struct GivenPicture {
    /// The image, base64, checked to draw.
    pub image: String,
    /// The path it was read from, when [`rustgis_map::layout::picture_source_kept`] keeps it.
    pub source: Option<String>,
    /// Read from a file whose path isn't kept: said once by the command.
    pub unremembered: bool,
}

/// A command's `path`: none when absent or null, as given when it is text that isn't blank and
/// isn't an address; refused otherwise.
pub(crate) fn picture_path_param(p: &Value) -> Result<Option<&str>, String> {
    match p.get("path") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(path)) if path.trim().is_empty() => Err(PICTURE_PATH.into()),
        Some(Value::String(path)) if path.contains("://") => Err(PICTURE_ADDRESS.into()),
        Some(Value::String(path)) => Ok(Some(path)),
        Some(_) => Err(PICTURE_PATH.into()),
    }
}

/// Whether a command gives a picture an `image` or a `path` (present and not null).
pub(crate) fn gives_picture(p: &Value) -> bool {
    ["image", "path"].iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null()))
}

/// The picture a command's `image` or `path` gives, none when it gives neither; refused when both
/// are given, either is of the wrong kind, the file can't be read or isn't a PNG or JPEG of at most
/// 16 MiB, or the image can't be drawn. Reads the file on the command's thread, as
/// `layout.addElement` always has: at most 16 MiB, then decoded once to check that it draws (the
/// decoded copy is kept for the page).
pub(in crate::cmd::layout) fn picture_given(p: &Value) -> Result<Option<GivenPicture>, String> {
    let given = |k: &str| p.get(k).filter(|v| !v.is_null());
    if given("image").is_some() && given("path").is_some() {
        return Err(PICTURE_BOTH.into());
    }
    let picture = match (given("image"), picture_path_param(p)?) {
        (Some(Value::String(image)), _) => {
            GivenPicture { image: crate::cmd::graphics::picture(&json!({"image": image}))?, source: None, unremembered: false }
        }
        (Some(_), _) => return Err(PICTURE_IMAGE.into()),
        (None, Some(path)) => {
            let image = crate::cmd::graphics::picture_file(path)?;
            let kept = rustgis_map::layout::picture_source_kept(path);
            GivenPicture { image, source: kept.then(|| path.to_string()), unremembered: !kept }
        }
        (None, None) => return Ok(None),
    };
    if !rustgis_render::layout::picture_draws(&picture.image) {
        return Err(picture_undrawn());
    }
    Ok(Some(picture))
}
