//! The shared daemon state: bar store plus UI events produced by requests.

use bossbar_proto::{Request, WireResponse};

use crate::bars::{BarStore, DEFAULT_FAIL_HOLD, DEFAULT_FINISH_HOLD};

/// Side effects the UI thread must act on after a request mutated the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEvent {
    /// Re-resolve the active monitor and move the pill.
    Reposition,
    /// Close the daemon.
    Shutdown,
}

#[derive(Default)]
pub struct Daemon {
    pub store: BarStore,
    events: Vec<UiEvent>,
    shutdown: bool,
    pub revision: u64,
}

impl Daemon {
    /// Applies one request. Never fails at the envelope level: errors are
    /// reported inside [`WireResponse`].
    pub fn apply(&mut self, request: &Request) -> WireResponse {
        let result: Result<WireResponse, String> = match request {
            Request::Ping => Ok(WireResponse::ok()),
            Request::List => Ok(WireResponse::with_data(self.store.snapshot())),
            Request::Create { id, bar } => match self.store.create(id.clone(), bar.clone()) {
                Ok(id) => {
                    self.push_event(UiEvent::Reposition);
                    Ok(WireResponse::with_data(serde_json::json!({ "id": id })))
                }
                Err(error) => Err(error),
            },
            Request::Update { id, patch } => self
                .store
                .update(id, patch.clone())
                .map(|()| WireResponse::ok()),
            Request::Tick { id, by, to } => {
                self.store.tick(id, *by, *to).map(|()| WireResponse::ok())
            }
            Request::Finish { id, hold_ms } => {
                let hold = hold_ms.map_or(DEFAULT_FINISH_HOLD, std::time::Duration::from_millis);
                self.store.finish(id, hold).map(|()| WireResponse::ok())
            }
            Request::Fail {
                id,
                message,
                hold_ms,
            } => {
                let hold = hold_ms.map_or(DEFAULT_FAIL_HOLD, std::time::Duration::from_millis);
                self.store
                    .fail(id, message.clone(), hold)
                    .map(|()| WireResponse::ok())
            }
            Request::Remove { id } => self.store.remove(id).map(|()| WireResponse::ok()),
            Request::Clear => {
                self.store.clear();
                Ok(WireResponse::ok())
            }
            Request::Collapse { value } => {
                self.store.set_collapsed(*value);
                Ok(WireResponse::ok())
            }
            Request::SetPosition { anchor } => {
                self.store.set_anchor(*anchor);
                self.push_event(UiEvent::Reposition);
                Ok(WireResponse::ok())
            }
            Request::SetPadding { value } => {
                self.store.set_padding(*value);
                self.push_event(UiEvent::Reposition);
                Ok(WireResponse::ok())
            }
            Request::SetVisible { value } => {
                let was_visible = self.store.visible;
                self.store.set_visible(*value);
                if self.store.visible && !was_visible {
                    self.push_event(UiEvent::Reposition);
                }
                Ok(WireResponse::ok())
            }
            Request::Shutdown => {
                self.shutdown = true;
                self.push_event(UiEvent::Shutdown);
                Ok(WireResponse::ok())
            }
        };
        self.revision = self.revision.wrapping_add(1);

        match result {
            Ok(response) => response,
            Err(error) => WireResponse::error(error),
        }
    }

    pub fn take_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    fn push_event(&mut self, event: UiEvent) {
        if !self.events.contains(&event) {
            self.events.push(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bossbar_proto::{Anchor, BarInit, BarKind, BarPatch, BarState, BarStatus, Request};

    fn create(daemon: &mut Daemon, label: &str) -> String {
        let response = daemon.apply(&Request::Create {
            id: None,
            bar: BarInit::new(label),
        });
        let value: serde_json::Value = response.into_data().unwrap();
        value["id"].as_str().unwrap().to_owned()
    }

    #[test]
    fn create_reports_id_and_requests_reposition() {
        let mut daemon = Daemon::default();
        let id = create(&mut daemon, "build");
        assert_eq!(id, "b1");
        assert!(daemon.take_events().contains(&UiEvent::Reposition));
    }

    #[test]
    fn update_finish_and_remove_flow() {
        let mut daemon = Daemon::default();
        let id = create(&mut daemon, "build");

        let response = daemon.apply(&Request::Update {
            id: id.clone(),
            patch: BarPatch {
                percent: Some(37.5),
                ..Default::default()
            },
        });
        assert!(response.ok);

        let response = daemon.apply(&Request::Finish {
            id: id.clone(),
            hold_ms: Some(0),
        });
        assert!(response.ok);
        assert_eq!(daemon.store.bars()[0].status, BarStatus::Done);

        let response = daemon.apply(&Request::Remove { id: id.clone() });
        assert!(response.ok);
        assert_eq!(daemon.store.live_count(), 0);
    }

    #[test]
    fn errors_are_reported_without_panicking() {
        let mut daemon = Daemon::default();
        let response = daemon.apply(&Request::Update {
            id: "missing".into(),
            patch: BarPatch {
                percent: Some(1.0),
                ..Default::default()
            },
        });
        assert!(!response.ok);
        assert!(response.error.unwrap().contains("missing"));
    }

    #[test]
    fn ticks_bars_reject_percent_updates_on_tick_path() {
        let mut daemon = Daemon::default();
        let response = daemon.apply(&Request::Create {
            id: Some("compile".into()),
            bar: BarInit {
                kind: BarKind::Ticks { total: 4 },
                ..BarInit::new("compile")
            },
        });
        assert!(response.ok);
        let response = daemon.apply(&Request::Tick {
            id: "compile".into(),
            by: None,
            to: Some(2),
        });
        assert!(response.ok);
        let state: BarState = daemon.apply(&Request::List).into_data().unwrap();
        assert_eq!(state.bars[0].tick, 2);
    }

    #[test]
    fn position_and_padding_changes_request_reposition() {
        let mut daemon = Daemon::default();
        daemon.take_events();

        daemon.apply(&Request::SetPosition {
            anchor: Anchor::TopRight,
        });
        assert!(daemon.take_events().contains(&UiEvent::Reposition));

        daemon.apply(&Request::SetPadding { value: Some(true) });
        assert!(daemon.store.snapshot().padding);
        assert!(daemon.take_events().contains(&UiEvent::Reposition));
    }

    #[test]
    fn visibility_toggle_requests_reposition_only_when_reappearing() {
        let mut daemon = Daemon::default();
        daemon.take_events();
        daemon.apply(&Request::SetVisible { value: Some(false) });
        assert!(daemon.take_events().is_empty());

        daemon.apply(&Request::SetVisible { value: Some(true) });
        assert!(daemon.take_events().contains(&UiEvent::Reposition));
    }
}
