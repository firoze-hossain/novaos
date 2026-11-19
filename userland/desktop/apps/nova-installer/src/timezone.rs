//! userland/desktop/apps/nova-installer/src/timezone.rs
//!
//! Timezone & Regional Localization Selection for NovaOS Graphical Installer.
//! Features:
//! - Worldwide database of major timezones across Americas, Europe, Asia, Africa, Oceania
//! - Search & autocomplete filtering
//! - Real-time UTC offset and simulated local clock preview
//! - Generates standard /etc/timezone and /etc/localtime configurations

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimezoneEntry {
    pub id: String,           // e.g. "America/New_York"
    pub region: String,       // e.g. "America"
    pub city: String,         // e.g. "New York"
    pub utc_offset_hours: i32,
    pub utc_offset_minutes: i32,
    pub abbreviation: String, // e.g. "EST" / "EDT"
}

impl TimezoneEntry {
    pub fn new(id: &str, region: &str, city: &str, offset_hours: i32, offset_mins: i32, abbr: &str) -> Self {
        Self {
            id: id.to_string(),
            region: region.to_string(),
            city: city.to_string(),
            utc_offset_hours: offset_hours,
            utc_offset_minutes: offset_mins,
            abbreviation: abbr.to_string(),
        }
    }

    pub fn formatted_offset(&self) -> String {
        let sign = if self.utc_offset_hours >= 0 && self.utc_offset_minutes >= 0 {
            "+"
        } else {
            "-"
        };
        format!(
            "UTC{}{:02}:{:02} ({})",
            sign,
            self.utc_offset_hours.abs(),
            self.utc_offset_minutes.abs(),
            self.abbreviation
        )
    }

    pub fn display_name(&self) -> String {
        format!("{} - {}", self.city, self.formatted_offset())
    }

    pub fn simulated_time_str(&self, utc_hour: u32, utc_minute: u32) -> String {
        let total_mins = (utc_hour as i32 * 60 + utc_minute as i32)
            + (self.utc_offset_hours * 60 + self.utc_offset_minutes);
        let normalized = (total_mins % 1440 + 1440) % 1440;
        let h = normalized / 60;
        let m = normalized % 60;
        let ampm = if h >= 12 { "PM" } else { "AM" };
        let h12 = if h == 0 { 12 } else if h > 12 { h - 12 } else { h };
        format!("{:02}:{:02} {}", h12, m, ampm)
    }
}

pub struct TimezoneDatabase {
    pub entries: Vec<TimezoneEntry>,
    pub selected_idx: usize,
}

impl TimezoneDatabase {
    pub fn new() -> Self {
        let mut db = Vec::new();

        // UTC
        db.push(TimezoneEntry::new("UTC", "Etc", "Universal Time (UTC)", 0, 0, "UTC"));

        // Americas
        db.push(TimezoneEntry::new("America/New_York", "Americas", "New York (Eastern)", -5, 0, "EST"));
        db.push(TimezoneEntry::new("America/Chicago", "Americas", "Chicago (Central)", -6, 0, "CST"));
        db.push(TimezoneEntry::new("America/Denver", "Americas", "Denver (Mountain)", -7, 0, "MST"));
        db.push(TimezoneEntry::new("America/Los_Angeles", "Americas", "Los Angeles (Pacific)", -8, 0, "PST"));
        db.push(TimezoneEntry::new("America/Toronto", "Americas", "Toronto", -5, 0, "EST"));
        db.push(TimezoneEntry::new("America/Vancouver", "Americas", "Vancouver", -8, 0, "PST"));
        db.push(TimezoneEntry::new("America/Sao_Paulo", "Americas", "São Paulo", -3, 0, "BRT"));
        db.push(TimezoneEntry::new("America/Buenos_Aires", "Americas", "Buenos Aires", -3, 0, "ART"));

        // Europe
        db.push(TimezoneEntry::new("Europe/London", "Europe", "London (GMT/BST)", 0, 0, "GMT"));
        db.push(TimezoneEntry::new("Europe/Paris", "Europe", "Paris (CET)", 1, 0, "CET"));
        db.push(TimezoneEntry::new("Europe/Berlin", "Europe", "Berlin (CET)", 1, 0, "CET"));
        db.push(TimezoneEntry::new("Europe/Rome", "Europe", "Rome (CET)", 1, 0, "CET"));
        db.push(TimezoneEntry::new("Europe/Madrid", "Europe", "Madrid (CET)", 1, 0, "CET"));
        db.push(TimezoneEntry::new("Europe/Amsterdam", "Europe", "Amsterdam (CET)", 1, 0, "CET"));
        db.push(TimezoneEntry::new("Europe/Stockholm", "Europe", "Stockholm (CET)", 1, 0, "CET"));
        db.push(TimezoneEntry::new("Europe/Warsaw", "Europe", "Warsaw (CET)", 1, 0, "CET"));

        // Asia & Middle East
        db.push(TimezoneEntry::new("Asia/Tokyo", "Asia", "Tokyo (JST)", 9, 0, "JST"));
        db.push(TimezoneEntry::new("Asia/Shanghai", "Asia", "Shanghai / Beijing (CST)", 8, 0, "CST"));
        db.push(TimezoneEntry::new("Asia/Hong_Kong", "Asia", "Hong Kong (HKT)", 8, 0, "HKT"));
        db.push(TimezoneEntry::new("Asia/Singapore", "Asia", "Singapore (SGT)", 8, 0, "SGT"));
        db.push(TimezoneEntry::new("Asia/Seoul", "Asia", "Seoul (KST)", 9, 0, "KST"));
        db.push(TimezoneEntry::new("Asia/Dhaka", "Asia", "Dhaka (BST)", 6, 0, "BST"));
        db.push(TimezoneEntry::new("Asia/Kolkata", "Asia", "Kolkata / Mumbai (IST)", 5, 30, "IST"));
        db.push(TimezoneEntry::new("Asia/Dubai", "Asia", "Dubai (GST)", 4, 0, "GST"));

        // Oceania & Australia
        db.push(TimezoneEntry::new("Australia/Sydney", "Oceania", "Sydney (AEST)", 10, 0, "AEST"));
        db.push(TimezoneEntry::new("Australia/Melbourne", "Oceania", "Melbourne (AEST)", 10, 0, "AEST"));
        db.push(TimezoneEntry::new("Australia/Perth", "Oceania", "Perth (AWST)", 8, 0, "AWST"));
        db.push(TimezoneEntry::new("Pacific/Auckland", "Oceania", "Auckland (NZST)", 12, 0, "NZST"));

        // Africa
        db.push(TimezoneEntry::new("Africa/Cairo", "Africa", "Cairo (EET)", 2, 0, "EET"));
        db.push(TimezoneEntry::new("Africa/Johannesburg", "Africa", "Johannesburg (SAST)", 2, 0, "SAST"));

        let default_idx = db.iter().position(|e| e.id == "America/New_York").unwrap_or(0);

        Self {
            entries: db,
            selected_idx: default_idx,
        }
    }

    pub fn selected(&self) -> &TimezoneEntry {
        &self.entries[self.selected_idx]
    }

    pub fn select_by_id(&mut self, id: &str) -> bool {
        if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
            self.selected_idx = pos;
            true
        } else {
            false
        }
    }

    /// Searches timezones matching query string
    pub fn search(&self, query: &str) -> Vec<&TimezoneEntry> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.entries.iter().collect();
        }

        self.entries
            .iter()
            .filter(|e| {
                e.id.to_lowercase().contains(&q)
                    || e.city.to_lowercase().contains(&q)
                    || e.region.to_lowercase().contains(&q)
                    || e.abbreviation.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Generates /etc/timezone string
    pub fn generate_etc_timezone(&self) -> String {
        format!("{}\n", self.selected().id)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timezone_database_and_search() {
        let mut db = TimezoneDatabase::new();
        assert!(!db.entries.is_empty());

        let ny = db.search("New York");
        assert_eq!(ny.len(), 1);
        assert_eq!(ny[0].id, "America/New_York");
        assert_eq!(ny[0].utc_offset_hours, -5);

        let tokyo = db.search("Tokyo");
        assert_eq!(tokyo.len(), 1);
        assert_eq!(tokyo[0].id, "Asia/Tokyo");
        assert_eq!(tokyo[0].utc_offset_hours, 9);

        // Selection
        assert!(db.select_by_id("Asia/Tokyo"));
        assert_eq!(db.selected().city, "Tokyo (JST)");

        // Simulated clock
        let time_str = db.selected().simulated_time_str(12, 0); // 12:00 UTC -> 21:00 Tokyo
        assert_eq!(time_str, "09:00 PM");

        assert_eq!(db.generate_etc_timezone(), "Asia/Tokyo\n");
    }
}
