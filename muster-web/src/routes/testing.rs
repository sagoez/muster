use super::site::Site;
use crate::{
    floor::Floor,
    http::Gate,
    live::{Hub, HubHandle},
};

/// A site behind `gate` whose hub owns `floor`, started on a thread of its
/// own, and the handle that feeds that hub.
pub(crate) fn live_site(gate: Gate, floor: Floor) -> (Site, HubHandle) {
    let (hub, handle) = Hub::new(floor);
    hub.spawn().unwrap();
    let site = Site::builder().gate(gate).hub(handle.clone()).build();
    (site, handle)
}
