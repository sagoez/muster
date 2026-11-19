use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::stack::events::alerts_view::AlertsView;

/// A result `view` carrying, beside its own fields, the alerts the call
/// heard.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Alerted<View> {
    /// The result itself.
    #[serde(flatten)]
    view: View,
    /// The alerts this connection had not heard yet.
    #[serde(flatten)]
    alerts: AlertsView,
}

impl<View> Alerted<View> {
    /// `view` with `alerts` beside it.
    pub fn of(view: View, alerts: &AlertsView) -> Self {
        Self::builder().view(view).alerts(alerts.clone()).build()
    }
}
