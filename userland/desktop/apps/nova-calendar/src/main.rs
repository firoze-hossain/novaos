//! userland/desktop/apps/nova-calendar/src/main.rs
//!
//! Entry Point for NovaOS Calendar (nova-calendar).

use std::env;
use std::process;

use nova_calendar::model::{CalendarEvent, CalendarState, EventCategory};
use nova_calendar::run_nova_calendar_selftest;

fn print_usage() {
    println!("NovaOS 2.0 Unified Calendar & Event Scheduler (nova-calendar)");
    println!("Usage:");
    println!("  nova-calendar [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --selftest                  Execute autonomous qualification test suite");
    println!("  --events, -e                List all scheduled events for active date");
    println!("  --month, -m                 Display terminal monthly calendar grid");
    println!("  --add <title> <time> <cat>  Add a new schedule entry to current date");
    println!("  --help, -h                  Show this help reference");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut state = CalendarState::new(2026, 11, 19);

    if args.len() > 1 {
        match args[1].as_str() {
            "--selftest" => {
                println!("[*] Running NovaOS Calendar Master Self-Test...");
                if run_nova_calendar_selftest() {
                    println!("[PASS] NovaOS Calendar self-test succeeded (100% checks passed).");
                    process::exit(0);
                } else {
                    eprintln!("[FAIL] NovaOS Calendar self-test failed.");
                    process::exit(1);
                }
            }
            "--events" | "-e" => {
                println!("=== NovaOS Calendar - Events for {} {}, {} ===", state.month_name(), state.today_day, state.today_year);
                let events = state.events_for_selected_date();
                if events.is_empty() {
                    println!("No scheduled events for this day.");
                } else {
                    for ev in events {
                        println!("  • [{}] {} (Time: {} - {}) @ {}", ev.category.name(), ev.title, ev.start_time, ev.end_time, ev.location);
                        println!("    {}", ev.description);
                    }
                }
                process::exit(0);
            }
            "--month" | "-m" => {
                println!("=== {} {} ===", state.month_name(), state.view_year);
                println!(" Su  Mo  Tu  We  Th  Fr  Sa");
                let first_dow = state.first_day_of_week() as usize;
                let total_d = state.days_in_current_month() as usize;

                for _ in 0..first_dow {
                    print!("    ");
                }
                for d in 1..=total_d {
                    let dow = (first_dow + d - 1) % 7;
                    if d as u8 == state.today_day {
                        print!("[{:2}]", d);
                    } else {
                        print!(" {:2} ", d);
                    }
                    if dow == 6 || d == total_d {
                        println!();
                    }
                }
                process::exit(0);
            }
            "--add" => {
                if args.len() >= 5 {
                    let title = &args[2];
                    let time = &args[3];
                    let cat_str = args[4].to_lowercase();
                    let cat = match cat_str.as_str() {
                        "personal" => EventCategory::Personal,
                        "novaos" | "release" => EventCategory::NovaOS,
                        "holiday" => EventCategory::Holiday,
                        "reminder" | "task" => EventCategory::Reminder,
                        _ => EventCategory::Work,
                    };
                    let id = state.add_event(CalendarEvent::new(
                        0,
                        title,
                        state.today_year,
                        state.today_month,
                        state.today_day,
                        time,
                        time,
                        cat,
                        "Local Workspace",
                        "Added via nova-calendar CLI",
                    ));
                    println!("[OK] Added event #{} '{}' at {} under {}.", id, title, time, cat.name());
                } else {
                    eprintln!("Usage: nova-calendar --add <title> <time> <category>");
                }
                process::exit(0);
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            other => {
                eprintln!("nova-calendar: unrecognized argument '{}'", other);
                print_usage();
                process::exit(1);
            }
        }
    }

    println!("=== NovaOS Calendar App (v2.0) ===");
    println!("Today: Thursday, November 19, 2026 (Timezone: {})", state.timezone_name);
    println!("Scheduled Events: {}", state.events.len());
    for ev in state.events_for_selected_date() {
        println!("  • [{}] {} ({})", ev.category.name(), ev.title, ev.start_time);
    }
    println!("Run with --help for options or --selftest for automated verification.");
}
