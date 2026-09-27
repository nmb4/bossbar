//! Wire format shared by the bossbar daemon and CLI.
//!
//! The daemon listens on a loopback TCP port and exchanges newline-delimited
//! JSON: one [`WireRequest`] in, one [`WireResponse`] out. Everything both
//! sides need to agree on lives here so the CLI can be built (and tested)
//! without pulling in the GUI stack.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Bumped whenever the request/response shape changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 1;

/// Biggest number of ticks a single bar may be split into.
pub const MAX_TICKS: u32 = 60;

/// Where the pill is anchored on the active monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    TopCenter,
    TopRight,
}

impl Anchor {
    pub const ALL: [Self; 2] = [Self::TopCenter, Self::TopRight];

    pub fn id(self) -> &'static str {
        match self {
            Self::TopCenter => "top-center",
            Self::TopRight => "top-right",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TopCenter => "Top center",
            Self::TopRight => "Top right",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "top-center" | "center" | "top" => Some(Self::TopCenter),
            "top-right" | "right" => Some(Self::TopRight),
            _ => None,
        }
    }
}

impl std::fmt::Display for Anchor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.id())
    }
}

/// How a bar expresses progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum BarKind {
    /// Smooth 0-100% bar.
    #[default]
    Percent,
    /// Minecraft-style segmented bar with `total` ticks.
    Ticks { total: u32 },
    /// Unknown duration: animated sheen, no meaningful percentage.
    Indeterminate,
}

impl BarKind {
    pub fn is_determinate(self) -> bool {
        !matches!(self, Self::Indeterminate)
    }
}

/// Lifecycle marker that changes the pill's accent treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BarStatus {
    #[default]
    Running,
    Done,
    Failed,
}

/// Fields accepted when a bar is first created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarInit {
    pub label: String,
    #[serde(default)]
    pub kind: BarKind,
    /// Percentage in `0..=100` for [`BarKind::Percent`].
    #[serde(default)]
    pub percent: f64,
    /// Current tick for [`BarKind::Ticks`]; `0..=total`.
    #[serde(default)]
    pub tick: u32,
    /// Optional second line of context shown under the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Optional color name (`blue`) or hex (`#6e9bff`). `None` uses the theme accent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl BarInit {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind: BarKind::Percent,
            percent: 0.0,
            tick: 0,
            detail: None,
            color: None,
        }
    }
}

/// Fields a running bar accepts as an update. `None` leaves a field alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BarPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<BarKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<BarStatus>,
}

/// Serializable view of one bar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarSnapshot {
    pub id: String,
    pub label: String,
    pub kind: BarKind,
    pub percent: f64,
    pub tick: u32,
    pub status: BarStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl BarSnapshot {
    /// Progress in `0.0..=1.0`; `None` for indeterminate bars.
    pub fn fraction(&self) -> Option<f32> {
        match self.kind {
            BarKind::Percent => Some((self.percent / 100.0).clamp(0.0, 1.0) as f32),
            BarKind::Ticks { total } => {
                if total == 0 {
                    Some(0.0)
                } else {
                    Some((self.tick.min(total) as f32 / total as f32).clamp(0.0, 1.0))
                }
            }
            BarKind::Indeterminate => None,
        }
    }

    /// Right-aligned value label, e.g. `47%` or `7/20`.
    pub fn value_text(&self) -> String {
        match self.kind {
            BarKind::Percent => format!("{}%", self.percent.round() as i64),
            BarKind::Ticks { total } => format!("{}/{}", self.tick.min(total), total),
            BarKind::Indeterminate => String::new(),
        }
    }
}

/// Complete daemon state as reported to the CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarState {
    pub bars: Vec<BarSnapshot>,
    pub collapsed: bool,
    pub anchor: Anchor,
    pub visible: bool,
    /// `false` (default) keeps the pill flush with the screen bounds;
    /// `true` adds breathing room around it.
    #[serde(default)]
    pub padding: bool,
}

impl Default for BarState {
    fn default() -> Self {
        Self {
            bars: Vec::new(),
            collapsed: false,
            anchor: Anchor::default(),
            visible: true,
            padding: false,
        }
    }
}

/// A single command sent by the CLI or the tray.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    /// Liveness probe.
    Ping,
    /// Current [`BarState`].
    List,
    /// Add a bar. Returns the assigned id.
    Create {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        bar: BarInit,
    },
    /// Patch a running bar.
    Update { id: String, patch: BarPatch },
    /// Advance a ticks bar, either by a delta or to an absolute tick.
    Tick {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<u32>,
    },
    /// Mark a bar complete: fills it, shows the success state, then removes it.
    Finish {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold_ms: Option<u64>,
    },
    /// Mark a bar failed: shows the failure state, then removes it.
    Fail {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold_ms: Option<u64>,
    },
    /// Remove a bar immediately (with a short exit animation).
    Remove { id: String },
    /// Remove every bar.
    Clear,
    /// Collapse or expand the pill. `None` toggles.
    Collapse {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<bool>,
    },
    /// Move the pill to another anchor.
    SetPosition { anchor: Anchor },
    /// Add or remove the padding around the pill. `None` toggles.
    SetPadding {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<bool>,
    },
    /// Show or hide the pill. `None` toggles.
    SetVisible {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<bool>,
    },
    /// Stop the daemon.
    Shutdown,
}

impl Request {
    /// Short name used in logs and error messages.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::List => "list",
            Self::Create { .. } => "create",
            Self::Update { .. } => "update",
            Self::Tick { .. } => "tick",
            Self::Finish { .. } => "finish",
            Self::Fail { .. } => "fail",
            Self::Remove { .. } => "remove",
            Self::Clear => "clear",
            Self::Collapse { .. } => "collapse",
            Self::SetPosition { .. } => "set-position",
            Self::SetPadding { .. } => "set-padding",
            Self::SetVisible { .. } => "set-visible",
            Self::Shutdown => "shutdown",
        }
    }
}

/// Envelope for a request: [`WireRequest::cmd`] is the actual command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireRequest {
    pub token: String,
    pub cmd: Request,
}

/// Envelope for a response. `data` carries [`BarState`] lists or created ids.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl WireResponse {
    pub fn ok() -> Self {
        Self {
            ok: true,
            data: None,
            error: None,
        }
    }

    pub fn with_data(data: impl Serialize) -> Self {
        Self {
            ok: true,
            data: serde_json::to_value(data).ok(),
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(message.into()),
        }
    }

    pub fn into_data<T: for<'de> Deserialize<'de>>(self) -> Result<T, String> {
        if !self.ok {
            return Err(self
                .error
                .unwrap_or_else(|| "unknown daemon error".to_owned()));
        }
        match self.data {
            Some(value) => serde_json::from_value(value)
                .map_err(|error| format!("unexpected daemon payload: {error}")),
            None => Err("daemon returned no payload".to_owned()),
        }
    }
}

/// Connection details written by the daemon once it starts listening.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub protocol: u32,
}

/// Per-user directory holding the daemon info, lock file, and logs.
pub fn state_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|root| root.join("bossbar"))
}

pub fn daemon_info_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("daemon.json"))
}

pub fn lock_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("daemon.lock"))
}

pub fn log_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("bossbar-daemon.log"))
}

/// Named bar colors available through `--color`.
pub const NAMED_COLORS: [(&str, [u8; 3]); 8] = [
    ("white", [0xed, 0xed, 0xf2]),
    ("blue", [0x6e, 0x9b, 0xff]),
    ("cyan", [0x58, 0xc7, 0xd6]),
    ("green", [0x6f, 0xcf, 0x97]),
    ("amber", [0xe8, 0xb4, 0x5e]),
    ("red", [0xe5, 0x48, 0x4d]),
    ("violet", [0xa7, 0x8b, 0xfa]),
    ("pink", [0xf4, 0x72, 0xb6]),
];

/// Parses `white`, `blue`, `#rgb`, or `#rrggbb` into RGB bytes.
pub fn parse_color(value: &str) -> Option<[u8; 3]> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    let lower = value.to_ascii_lowercase();
    NAMED_COLORS
        .iter()
        .find(|(name, _)| *name == lower)
        .map(|(_, rgb)| *rgb)
}

fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    let expanded = match hex.len() {
        3 => hex
            .chars()
            .flat_map(|character| [character, character])
            .collect::<String>(),
        6 => hex.to_owned(),
        _ => return None,
    };
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&expanded[range], 16).ok();
    Some([channel(0..2)?, channel(2..4)?, channel(4..6)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_fraction_clamps() {
        let bar = BarSnapshot {
            id: "b1".into(),
            label: "build".into(),
            kind: BarKind::Percent,
            percent: 140.0,
            tick: 0,
            status: BarStatus::Running,
            detail: None,
            color: None,
        };
        assert_eq!(bar.fraction(), Some(1.0));
        assert_eq!(bar.value_text(), "140%");
    }

    #[test]
    fn ticks_fraction_uses_total() {
        let bar = BarSnapshot {
            id: "b1".into(),
            label: "compile".into(),
            kind: BarKind::Ticks { total: 20 },
            percent: 0.0,
            tick: 7,
            status: BarStatus::Running,
            detail: None,
            color: None,
        };
        assert_eq!(bar.fraction(), Some(0.35));
        assert_eq!(bar.value_text(), "7/20");
    }

    #[test]
    fn indeterminate_has_no_fraction() {
        let bar = BarSnapshot {
            id: "b1".into(),
            label: "thinking".into(),
            kind: BarKind::Indeterminate,
            percent: 0.0,
            tick: 0,
            status: BarStatus::Running,
            detail: None,
            color: None,
        };
        assert_eq!(bar.fraction(), None);
        assert_eq!(bar.value_text(), "");
    }

    #[test]
    fn colors_parse_names_and_hex() {
        assert_eq!(parse_color("blue"), Some([0x6e, 0x9b, 0xff]));
        assert_eq!(parse_color("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_color("#6E9BFF"), Some([0x6e, 0x9b, 0xff]));
        assert_eq!(parse_color("chartreuse"), None);
        assert_eq!(parse_color("#12345"), None);
    }

    #[test]
    fn requests_round_trip_as_tagged_json() {
        let request = WireRequest {
            token: "secret".into(),
            cmd: Request::Create {
                id: Some("build".into()),
                bar: BarInit {
                    label: "Building".into(),
                    kind: BarKind::Ticks { total: 12 },
                    percent: 0.0,
                    tick: 3,
                    detail: Some("link".into()),
                    color: Some("blue".into()),
                },
            },
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"cmd\":\"create\""));
        assert!(json.contains("\"type\":\"ticks\""));
        let back: WireRequest = serde_json::from_str(&json).unwrap();
        matches!(back.cmd, Request::Create { .. });
    }

    #[test]
    fn response_payloads_decode() {
        let state = BarState {
            bars: vec![],
            collapsed: true,
            anchor: Anchor::TopRight,
            visible: true,
            padding: true,
        };
        let response = WireResponse::with_data(state.clone());
        let decoded: BarState = response.into_data().unwrap();
        assert_eq!(decoded, state);

        let failure = WireResponse::error("no such bar");
        assert!(failure.into_data::<BarState>().is_err());
    }

    #[test]
    fn anchor_round_trips() {
        for anchor in Anchor::ALL {
            assert_eq!(Anchor::parse(anchor.id()), Some(anchor));
            assert_eq!(Anchor::parse(&anchor.to_string()), Some(anchor));
        }
        assert_eq!(Anchor::parse("left"), None);
    }
}
