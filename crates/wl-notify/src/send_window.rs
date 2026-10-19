//! The stored send window of a channel (wl-notify §7, `NTF.RUL.02`, #62 A2).
//!
//! No row means the default 07:00-21:00, so an install without a saved
//! window behaves as before. A save is checked by [`window::validate`]; a
//! refused window changes nothing.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::notify::NotifyChannel;

use crate::window::{self, Window, WindowError};

/// One channel's marketing send window.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_notify_send_windows", unique(channel))]
pub struct SendWindow {
    /// The primary key.
    pub id: i64,
    /// The channel the window is for.
    pub channel: NotifyChannel,
    /// Minutes after midnight Asia/Dubai the window opens.
    pub start_minute: i64,
    /// Minutes after midnight Asia/Dubai the window closes.
    pub end_minute: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SendWindow {
    /// The window of this row.
    pub fn window(&self) -> Window {
        Window {
            start: u32::try_from(self.start_minute).unwrap_or(window::DEFAULT_START),
            end: u32::try_from(self.end_minute).unwrap_or(window::DEFAULT_END),
        }
    }

    /// The window of `channel`: the stored one, else the default.
    pub async fn for_channel(db: &Db, channel: NotifyChannel) -> Result<Window> {
        Ok(SendWindow::query()
            .where_(SendWindow::CHANNEL.eq(channel))
            .first(db)
            .await?
            .map_or_else(Window::default, |row| row.window()))
    }

    /// Saves `window` for `channel` after [`window::validate`]. A refused
    /// window is `Ok(Err(..))` and nothing is stored.
    pub async fn save_checked(
        db: &Db,
        channel: NotifyChannel,
        window: Window,
    ) -> Result<std::result::Result<SendWindow, WindowError>> {
        if let Err(refusal) = window::validate(channel, window) {
            return Ok(Err(refusal));
        }
        let existing = SendWindow::query()
            .where_(SendWindow::CHANNEL.eq(channel))
            .first(db)
            .await?;
        let mut row = existing.unwrap_or(SendWindow {
            channel,
            ..Default::default()
        });
        row.start_minute = i64::from(window.start);
        row.end_minute = i64::from(window.end);
        row.save(db).await?;
        Ok(Ok(row))
    }
}
