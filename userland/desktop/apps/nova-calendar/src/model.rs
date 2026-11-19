//! userland/desktop/apps/nova-calendar/src/model.rs
//!
//! Calendar Data Model, Recurring Event Engine, and State Management.
//! Synthesizes Apple Calendar (macOS), Windows 11 Calendar, and GNOME Calendar (Ubuntu).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventCategory {
    Work,       // Blue (macOS Work calendar)
    Personal,   // Green (macOS Personal / Family)
    NovaOS,     // Electric Cyan (NovaOS Milestones & Kernel Releases)
    Holiday,    // Amber (Public Holidays & Observances)
    Reminder,   // Purple (Tasks & Reminders)
}

impl EventCategory {
    pub fn all() -> [EventCategory; 5] {
        [
            EventCategory::Work,
            EventCategory::Personal,
            EventCategory::NovaOS,
            EventCategory::Holiday,
            EventCategory::Reminder,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            EventCategory::Work => "Work & Meetings",
            EventCategory::Personal => "Personal & Life",
            EventCategory::NovaOS => "NovaOS Releases",
            EventCategory::Holiday => "Holidays",
            EventCategory::Reminder => "Tasks & Reminders",
        }
    }

    pub fn color_rgb(&self) -> (u8, u8, u8) {
        match self {
            EventCategory::Work => (59, 130, 246),      // Royal Blue #3B82F6
            EventCategory::Personal => (34, 197, 94),   // Emerald Green #22C55E
            EventCategory::NovaOS => (6, 182, 212),     // Electric Cyan #06B6D4
            EventCategory::Holiday => (245, 158, 11),   // Amber Gold #F59E0B
            EventCategory::Reminder => (168, 85, 247),  // Violet Purple #A855F7
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Month,  // Full 7x5 or 7x6 month grid (Ubuntu GNOME + macOS style)
    Week,   // 7-day multi-column timeline
    Day,    // Hourly itinerary breakdown (Windows 11 schedule card)
    Agenda, // Chronological event feed (Windows 11 Focus / macOS list)
}

impl ViewMode {
    pub fn all() -> [ViewMode; 4] {
        [ViewMode::Month, ViewMode::Week, ViewMode::Day, ViewMode::Agenda]
    }

    pub fn name(&self) -> &'static str {
        match self {
            ViewMode::Month => "Month",
            ViewMode::Week => "Week",
            ViewMode::Day => "Day",
            ViewMode::Agenda => "Agenda",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarEvent {
    pub id: u32,
    pub title: String,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub start_time: String,
    pub end_time: String,
    pub category: EventCategory,
    pub location: String,
    pub description: String,
    pub is_completed: bool,
}

impl CalendarEvent {
    pub fn new(
        id: u32,
        title: impl Into<String>,
        year: u16,
        month: u8,
        day: u8,
        start_time: impl Into<String>,
        end_time: impl Into<String>,
        category: EventCategory,
        location: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            year,
            month,
            day,
            start_time: start_time.into(),
            end_time: end_time.into(),
            category,
            location: location.into(),
            description: description.into(),
            is_completed: false,
        }
    }
}

pub fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

pub fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 => 31,
        2 => if is_leap_year(year) { 29 } else { 28 },
        3 => 31,
        4 => 30,
        5 => 31,
        6 => 30,
        7 => 31,
        8 => 31,
        9 => 30,
        10 => 31,
        11 => 30,
        12 => 31,
        _ => 30,
    }
}

pub fn day_of_week(year: u16, month: u8, day: u8) -> u8 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year as i32;
    if month < 3 {
        y -= 1;
    }
    let m = (month.saturating_sub(1) as usize).min(11);
    let dow = (y + y / 4 - y / 100 + y / 400 + t[m] + day as i32) % 7;
    if dow < 0 {
        ((dow + 7) % 7) as u8
    } else {
        dow as u8
    }
}

pub const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];

pub const DAY_NAMES_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const DAY_NAMES_FULL: [&str; 7] = [
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
];

#[derive(Clone, Debug)]
pub struct CalendarState {
    pub today_year: u16,
    pub today_month: u8,
    pub today_day: u8,

    pub view_year: u16,
    pub view_month: u8,

    pub selected_year: u16,
    pub selected_month: u8,
    pub selected_day: u8,

    pub view_mode: ViewMode,
    pub timezone_name: String,
    pub events: Vec<CalendarEvent>,
    pub next_event_id: u32,

    pub enabled_categories: Vec<EventCategory>,
    pub search_query: String,
}

impl Default for CalendarState {
    fn default() -> Self {
        Self::new(2026, 11, 19)
    }
}

impl CalendarState {
    pub fn new(year: u16, month: u8, day: u8) -> Self {
        let mut state = Self {
            today_year: year,
            today_month: month,
            today_day: day,

            view_year: year,
            view_month: month,

            selected_year: year,
            selected_month: month,
            selected_day: day,

            view_mode: ViewMode::Month,
            timezone_name: "Asia/Dhaka (UTC+06:00)".to_string(),
            events: Vec::new(),
            next_event_id: 1,

            enabled_categories: EventCategory::all().to_vec(),
            search_query: String::new(),
        };

        state.load_default_enterprise_events();
        state
    }

    pub fn load_default_enterprise_events(&mut self) {
        let y = self.today_year;
        let m = self.today_month;

        self.add_event(CalendarEvent::new(
            self.next_event_id,
            "Daily Architecture Standup",
            y,
            m,
            self.today_day,
            "09:30 AM",
            "10:00 AM",
            EventCategory::Work,
            "Virtual / Matrix Channel",
            "Daily sync on kernel ring-3 transition, Long Mode memory paging, and SMP.",
        ));

        self.add_event(CalendarEvent::new(
            self.next_event_id,
            "NovaOS 2.0 GA Release Launch",
            y,
            m,
            self.today_day,
            "02:00 PM",
            "04:00 PM",
            EventCategory::NovaOS,
            "Main Auditorium & Webcast",
            "Official announcement and release signing of NovaOS 2.0 Enterprise Edition.",
        ));

        self.add_event(CalendarEvent::new(
            self.next_event_id,
            "Team Celebration Dinner",
            y,
            m,
            self.today_day,
            "07:30 PM",
            "09:30 PM",
            EventCategory::Personal,
            "The Sapphire Bistro",
            "Celebration of reaching Month 6 Release Hardening milestone.",
        ));

        // Upcoming events in the same month
        let next_d1 = (self.today_day + 1).min(days_in_month(y, m));
        self.add_event(CalendarEvent::new(
            self.next_event_id,
            "Kernel Security Audit Review",
            y,
            m,
            next_d1,
            "11:00 AM",
            "12:30 PM",
            EventCategory::Work,
            "Security Operations Center",
            "Review of dual 32/64-bit ABI boundaries and address space isolation.",
        ));

        let next_d2 = (self.today_day + 5).min(days_in_month(y, m));
        self.add_event(CalendarEvent::new(
            self.next_event_id,
            "Enterprise Thanksgiving Break",
            y,
            m,
            next_d2,
            "All Day",
            "All Day",
            EventCategory::Holiday,
            "Global",
            "Annual international enterprise thanksgiving holiday observation.",
        ));

        let next_d3 = (self.today_day + 8).min(days_in_month(y, m));
        self.add_event(CalendarEvent::new(
            self.next_event_id,
            "Sprint Retrospective & Roadmap v2.1",
            y,
            m,
            next_d3,
            "03:00 PM",
            "04:30 PM",
            EventCategory::Work,
            "Conference Room Alpha",
            "Review Month 6 deliverables and outline roadmap for upcoming patches.",
        ));

        self.add_event(CalendarEvent::new(
            self.next_event_id,
            "Backup Cloud Snapshot Verification",
            y,
            m,
            self.today_day,
            "11:00 PM",
            "11:30 PM",
            EventCategory::Reminder,
            "Automated Server Host",
            "Verify automated Btrfs snapshot replication across offsite cluster.",
        ));
    }

    pub fn add_event(&mut self, mut event: CalendarEvent) -> u32 {
        let id = self.next_event_id;
        event.id = id;
        self.next_event_id += 1;
        self.events.push(event);
        id
    }

    pub fn delete_event(&mut self, id: u32) -> bool {
        let len = self.events.len();
        self.events.retain(|e| e.id != id);
        self.events.len() < len
    }

    pub fn toggle_event_completed(&mut self, id: u32) -> bool {
        if let Some(ev) = self.events.iter_mut().find(|e| e.id == id) {
            ev.is_completed = !ev.is_completed;
            true
        } else {
            false
        }
    }

    pub fn select_date(&mut self, year: u16, month: u8, day: u8) {
        self.selected_year = year;
        self.selected_month = month;
        self.selected_day = day;
        self.view_year = year;
        self.view_month = month;
    }

    pub fn next_month(&mut self) {
        if self.view_month == 12 {
            self.view_month = 1;
            self.view_year += 1;
        } else {
            self.view_month += 1;
        }
        let max_d = days_in_month(self.view_year, self.view_month);
        self.selected_year = self.view_year;
        self.selected_month = self.view_month;
        self.selected_day = self.selected_day.min(max_d);
    }

    pub fn prev_month(&mut self) {
        if self.view_month == 1 {
            self.view_month = 12;
            self.view_year = self.view_year.saturating_sub(1);
        } else {
            self.view_month -= 1;
        }
        let max_d = days_in_month(self.view_year, self.view_month);
        self.selected_year = self.view_year;
        self.selected_month = self.view_month;
        self.selected_day = self.selected_day.min(max_d);
    }

    pub fn go_today(&mut self) {
        self.view_year = self.today_year;
        self.view_month = self.today_month;
        self.selected_year = self.today_year;
        self.selected_month = self.today_month;
        self.selected_day = self.today_day;
    }

    pub fn toggle_category(&mut self, cat: EventCategory) {
        if let Some(pos) = self.enabled_categories.iter().position(|&c| c == cat) {
            self.enabled_categories.remove(pos);
        } else {
            self.enabled_categories.push(cat);
        }
    }

    pub fn is_category_enabled(&self, cat: EventCategory) -> bool {
        self.enabled_categories.contains(&cat)
    }

    pub fn events_for_date(&self, year: u16, month: u8, day: u8) -> Vec<&CalendarEvent> {
        self.events
            .iter()
            .filter(|e| {
                e.year == year
                    && e.month == month
                    && e.day == day
                    && self.is_category_enabled(e.category)
            })
            .collect()
    }

    pub fn events_for_selected_date(&self) -> Vec<&CalendarEvent> {
        self.events_for_date(self.selected_year, self.selected_month, self.selected_day)
    }

    pub fn filtered_events(&self) -> Vec<&CalendarEvent> {
        self.events
            .iter()
            .filter(|e| self.is_category_enabled(e.category))
            .filter(|e| {
                if self.search_query.is_empty() {
                    true
                } else {
                    let q = self.search_query.to_lowercase();
                    e.title.to_lowercase().contains(&q)
                        || e.description.to_lowercase().contains(&q)
                        || e.location.to_lowercase().contains(&q)
                }
            })
            .collect()
    }

    pub fn month_name(&self) -> &'static str {
        let idx = (self.view_month.saturating_sub(1) as usize).min(11);
        MONTH_NAMES[idx]
    }

    pub fn days_in_current_month(&self) -> u8 {
        days_in_month(self.view_year, self.view_month)
    }

    pub fn first_day_of_week(&self) -> u8 {
        day_of_week(self.view_year, self.view_month, 1)
    }
}
