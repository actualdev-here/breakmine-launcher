//! Bounded, display-only game status passed over the host's stdout.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Snapshot {
    pub state: Option<String>,
    pub world: Option<String>,
    pub paused: bool,
    pub singleplayer: Option<bool>,
    pub connected: bool,
    pub players: Vec<String>,
}
impl Snapshot {
    pub fn parse(input: &str) -> Option<Self> {
        if input.len() > 16384 { return None; }
        let mut result: Self = serde_json::from_str(input).ok()?;
        if result.players.len() > 128 { return None; }
        fn clean(s: &mut String) { *s = s.chars().filter(|c| !c.is_control()).take(80).collect(); }
        if let Some(v) = &mut result.state { clean(v); }
        if let Some(v) = &mut result.world { clean(v); }
        for player in &mut result.players { clean(player); }
        if !result.connected { result.players.clear(); }
        Some(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_bounded_status_and_drops_disconnected_players() {
        let a = Snapshot::parse(r#"{"state":"playing","connected":true,"players":["Alex","Steve"]}"#).unwrap();
        assert_eq!(a.players, ["Alex", "Steve"]);
        let a = Snapshot::parse(r#"{"connected":false,"players":["Alex"]}"#).unwrap();
        assert!(a.players.is_empty());
        assert!(Snapshot::parse(r#"{"token":"secret"}"#).is_none());
        assert!(Snapshot::parse(&"a".repeat(16385)).is_none());
    }
}
