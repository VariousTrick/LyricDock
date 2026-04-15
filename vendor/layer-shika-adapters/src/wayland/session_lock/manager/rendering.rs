use crate::errors::Result;
use wayland_client::backend::ObjectId;

use super::state::ActiveLockSurface;

pub(super) fn render_frames(lock_surfaces: &[(ObjectId, ActiveLockSurface)]) -> Result<()> {
    for (_, surface) in lock_surfaces {
        surface.render_frame_if_dirty()?;
    }
    Ok(())
}
