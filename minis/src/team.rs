//! Which developer team signs (step 4). Apple lists every team an Apple
//! Account is on: the free one every account has, and a paid Apple Developer
//! Program membership too when there is one, its own or a company's. siboot
//! took the first until 0.19.0; now it asks when there's more than one, with
//! each marked free or paid, since that decides how long the app lasts.
//!
//! No isideload here, so `examples/screens.rs` shows the same question.

use std::collections::HashMap;

use serde::Deserialize;

/// What the question asks, while step 4 runs.
pub const QUESTION: &str = "sign with which team?";

/// What a team is, as far as signing goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The free team every Apple Account has (Xcode's Personal Team): what it
    /// signs lasts 7 days.
    Free,
    /// An Apple Developer Program membership, the account's own or a
    /// company's: what it signs lasts a year.
    Paid,
    /// Apple's listing didn't say, or wasn't read (one team only).
    Unknown,
}

impl Kind {
    /// From a team's `type` in Apple's listTeams and the names of its
    /// memberships, the way AltSign's `ALTTeam` reads them (AltStore and
    /// SideStore go by it): a company's or organization's team is paid; an
    /// individual's is free when its one membership is named as free, and
    /// paid otherwise. One with no membership listed isn't guessed at.
    pub fn of(team_type: Option<&str>, memberships: &[String]) -> Kind {
        match (team_type, memberships) {
            (Some("Company/Organization"), _) => Kind::Paid,
            (Some("Individual"), [only]) if only.to_lowercase().contains("free") => Kind::Free,
            (Some("Individual"), [_, ..]) => Kind::Paid,
            _ => Kind::Unknown,
        }
    }
}

/// A team in Apple's listTeams answer, with what isideload's `DeveloperTeam`
/// leaves out.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
    pub team_id: String,
    pub r#type: Option<String>,
    #[serde(default)]
    pub memberships: Vec<Membership>,
}

#[derive(Deserialize, Debug)]
pub struct Membership {
    #[serde(default)]
    pub name: String,
}

/// Each listed team's kind, by team ID.
pub fn kinds(listed: &[Listed]) -> HashMap<String, Kind> {
    listed
        .iter()
        .map(|team| {
            let names: Vec<String> = team.memberships.iter().map(|m| m.name.clone()).collect();
            (
                team.team_id.clone(),
                Kind::of(team.r#type.as_deref(), &names),
            )
        })
        .collect()
}

/// "Frizzle (ABCDE12345)", as siboot has always named the team.
pub fn named(name: Option<&str>, id: &str) -> String {
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("unnamed");
    format!("{name} ({id})")
}

/// The team as the question lists it, "Paid · Frizzle (ABCDE12345)": the
/// kind first, so a narrow screen that cuts the line still shows it.
pub fn label(name: Option<&str>, id: &str, kind: Kind) -> String {
    match kind {
        Kind::Free => format!("Free · {}", named(name, id)),
        Kind::Paid => format!("Paid · {}", named(name, id)),
        Kind::Unknown => named(name, id),
    }
}

/// The step's summary: "paid team Frizzle (ABCDE12345)".
pub fn summary(name: Option<&str>, id: &str, kind: Kind) -> String {
    match kind {
        Kind::Free => format!("free team {}", named(name, id)),
        Kind::Paid => format!("paid team {}", named(name, id)),
        Kind::Unknown => format!("team {}", named(name, id)),
    }
}

/// What the step shows while the question waits. Short enough for the live
/// screen's line on an upright iPhone.
pub fn found(teams: usize) -> String {
    format!("{teams} teams on this Apple Account")
}

/// Said before the question.
pub const NOTE: &str = "A free team's apps last 7 days; a paid team's last a year.";

/// The finish box's line on how long SideInstaller lasts. `first`: signed by
/// the first team Apple lists. SideInstaller renews itself with that one (its
/// rust-core takes the first), so it can renew itself in place only then;
/// otherwise the renewal is running siboot again with the same team.
pub fn lasts(kind: Kind, first: bool) -> &'static str {
    match (kind, first) {
        (Kind::Paid, _) => "A paid team signed it, so it lasts a year.",
        (_, true) => {
            "Free accounts' apps last 7 days; SideInstaller can renew itself from then on."
        }
        (_, false) => {
            "Free accounts' apps last 7 days. To renew it, run siboot again and pick the same team."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn free_and_paid_as_altsign_tells_them() {
        let individual = Some("Individual");
        let company = Some("Company/Organization");
        assert_eq!(
            Kind::of(individual, &names(&["Some Free Program"])),
            Kind::Free
        );
        assert_eq!(Kind::of(individual, &names(&["free"])), Kind::Free);
        assert_eq!(
            Kind::of(individual, &names(&["Apple Developer Program"])),
            Kind::Paid
        );
        // More than one membership: paid, even with a free one among them.
        assert_eq!(
            Kind::of(individual, &names(&["Free", "Apple Developer Program"])),
            Kind::Paid
        );
        assert_eq!(
            Kind::of(company, &names(&["Apple Developer Program"])),
            Kind::Paid
        );
        assert_eq!(Kind::of(company, &[]), Kind::Paid);
        // Nothing to go by.
        assert_eq!(Kind::of(individual, &[]), Kind::Unknown);
        assert_eq!(
            Kind::of(Some("Something Else"), &names(&["Free"])),
            Kind::Unknown
        );
        assert_eq!(Kind::of(None, &names(&["Free"])), Kind::Unknown);
    }

    /// A listTeams answer as isideload gets it, read the way its
    /// `send_dev_request` reads one (`plist::from_value` on "teams"). The
    /// membership names are made up: only "free" in them counts.
    #[test]
    fn kinds_come_from_a_list_teams_answer() {
        let answer = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>resultCode</key><integer>0</integer>
  <key>teams</key>
  <array>
    <dict>
      <key>name</key><string>Frizzle</string>
      <key>teamId</key><string>ABCDE12345</string>
      <key>type</key><string>Individual</string>
      <key>status</key><string>active</string>
      <key>dateCreated</key><date>2024-05-01T10:00:00Z</date>
      <key>memberships</key>
      <array>
        <dict>
          <key>name</key><string>Xcode Free Provisioning</string>
          <key>platform</key><string>ios</string>
          <key>dateStart</key><date>2024-05-01T10:00:00Z</date>
        </dict>
      </array>
    </dict>
    <dict>
      <key>name</key><string>Frizzle</string>
      <key>teamId</key><string>FGHIJ67890</string>
      <key>type</key><string>Individual</string>
      <key>memberships</key>
      <array>
        <dict><key>name</key><string>Apple Developer Program</string></dict>
      </array>
    </dict>
    <dict>
      <key>name</key><string>Example Ltd</string>
      <key>teamId</key><string>KLMNO24680</string>
      <key>type</key><string>Company/Organization</string>
    </dict>
    <dict>
      <key>teamId</key><string>PQRST13579</string>
      <key>type</key><string>Individual</string>
    </dict>
  </array>
</dict>
</plist>"#;
        let dict: plist::Dictionary = plist::from_bytes(answer).unwrap();
        let listed: Vec<Listed> = plist::from_value(dict.get("teams").unwrap()).unwrap();
        let kinds = kinds(&listed);
        assert_eq!(kinds["ABCDE12345"], Kind::Free);
        assert_eq!(kinds["FGHIJ67890"], Kind::Paid);
        assert_eq!(kinds["KLMNO24680"], Kind::Paid);
        assert_eq!(kinds["PQRST13579"], Kind::Unknown);
    }

    #[test]
    fn labels_lead_with_the_kind() {
        assert_eq!(
            label(Some("Frizzle"), "ABCDE12345", Kind::Free),
            "Free · Frizzle (ABCDE12345)"
        );
        assert_eq!(
            label(Some("Frizzle"), "FGHIJ67890", Kind::Paid),
            "Paid · Frizzle (FGHIJ67890)"
        );
        assert_eq!(
            label(Some("Frizzle"), "ABCDE12345", Kind::Unknown),
            "Frizzle (ABCDE12345)"
        );
        assert_eq!(
            label(None, "PQRST13579", Kind::Unknown),
            "unnamed (PQRST13579)"
        );
        assert_eq!(
            label(Some("  "), "PQRST13579", Kind::Free),
            "Free · unnamed (PQRST13579)"
        );
        assert_eq!(
            summary(Some("Frizzle"), "FGHIJ67890", Kind::Paid),
            "paid team Frizzle (FGHIJ67890)"
        );
        assert_eq!(
            summary(Some("Frizzle"), "ABCDE12345", Kind::Unknown),
            "team Frizzle (ABCDE12345)"
        );
    }

    /// The live screen's step line has 31 columns on a 45-column terminal,
    /// after the step's name.
    #[test]
    fn the_step_line_fits_an_upright_iphone() {
        assert_eq!(found(2), "2 teams on this Apple Account");
        assert!(found(9).chars().count() <= 31);
    }

    #[test]
    fn the_finish_box_says_how_long_it_lasts() {
        assert_eq!(lasts(Kind::Paid, false), lasts(Kind::Paid, true));
        assert!(lasts(Kind::Paid, true).contains("a year"));
        // The first team is the one SideInstaller renews itself with.
        assert!(lasts(Kind::Free, true).contains("renew itself"));
        assert!(lasts(Kind::Unknown, true).contains("renew itself"));
        assert!(lasts(Kind::Free, false).contains("run siboot again"));
        assert!(!lasts(Kind::Free, false).contains("renew itself"));
    }
}
