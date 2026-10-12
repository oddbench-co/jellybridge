//! Infrastructure services: media playback pipeline (mpv worker, token-hiding
//! loopback proxy, GL render bridge) and Jellyfin playback preparation.

pub mod artwork;
pub mod dns;
pub mod libmpv;
pub mod playback;
pub mod player;
pub mod proxy;
pub mod render;
pub mod session;
