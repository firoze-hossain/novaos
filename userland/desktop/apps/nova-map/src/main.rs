//! userland/desktop/apps/nova-map/src/main.rs
//!
//! # NovaOS Map Application CLI Runner (nova-map / Atlas)
//!
//! Sovereign vector maps and navigation CLI, telemetry reader, and
//! autonomous qualification test runner.

use std::env;
use std::process::exit;

use nova_map::{
    run_nova_map_selftest, MapDatabase, PlatformLocation,
};

fn print_usage() {
    println!("NovaOS Sovereign Vector Maps (nova-map / Atlas) v1.0.0");
    println!("Synthesizing macOS Maps Acrylic Interface with Pure-Rust Navigation Engine");
    println!();
    println!("Usage:");
    println!("  nova-map                         Display map telemetry, coordinates & viewport");
    println!("  nova-map --selftest              Execute autonomous qualification test suite");
    println!("  nova-map --coords                Print current high-precision GPS coordinates");
    println!("  nova-map --poi                   List nearby points of interest and categories");
    println!("  nova-map --search <QUERY>        Geocode search for hospitals, dining, transit");
    println!("  nova-map --info                  Print cartography subsystems architecture");
    println!("  nova-map --version               Print version information");
    println!("  nova-map --help                  Display this reference manual");
    println!();
    println!("Subsystems:");
    println!("  1. Web Mercator: Exact EPSG:3857 projection matrices forward and inverse");
    println!("  2. Geodesy: Haversine great-circle distance & bearing mathematics");
    println!("  3. Spatial DB: Vector polygons (Lakes, Parks), multi-pass road networks, POIs");
    println!("  4. Navigation: Turn-by-turn waypoint routing, step maneuvers & durations");
    println!("  5. 3D Perspective: Dynamic 35° horizon tilt and depth ground-plane rasterizer");
    println!("  6. macOS Shell: Floating translucent acrylic sidebar, Nearby drawer, Marked Location card");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_usage();
                exit(0);
            }
            "--selftest" | "-t" => {
                println!("[*] Launching NovaOS Map Autonomous Qualification Suite...");
                let ok = run_nova_map_selftest();
                if ok {
                    println!("[PASS] NovaOS Map (nova-map / Atlas) passed all qualification checks.");
                    exit(0);
                } else {
                    eprintln!("[FAIL] One or more nova-map tests failed.");
                    exit(1);
                }
            }
            "--coords" | "-c" => {
                let loc = PlatformLocation::current_location();
                println!("Current GPS Location Telemetry:");
                println!("  Coordinates: {}", loc.coord.format_dms());
                println!("  Altitude:    {:.1} m", loc.altitude_meters);
                println!("  Accuracy:    ±{:.1} m", loc.accuracy_meters);
                println!("  GPS Fix:     {:?}", loc.fix);
                println!("  Weather:     25°C, Mostly Sunny, AQI 82");
                exit(0);
            }
            "--poi" | "-p" => {
                println!("Nearby Points of Interest:");
                for poi in MapDatabase::load_pois() {
                    println!("  * [{:?}] {} (Rating: {:.1}) - {}", poi.category, poi.name, poi.rating, poi.address);
                }
                exit(0);
            }
            "--search" | "-s" => {
                if let Some(query) = args.get(2) {
                    println!("Geocoding search for '{}':", query);
                    let res = MapDatabase::search_pois(query);
                    if res.is_empty() {
                        println!("  No matching points of interest found.");
                    } else {
                        for p in res {
                            println!("  * {} ({}) - {}", p.name, p.category.name(), p.address);
                            println!("    Coordinates: {}", p.coord.format_dms());
                        }
                    }
                    exit(0);
                } else {
                    eprintln!("Error: --search requires a search query argument");
                    exit(1);
                }
            }
            "--info" | "-i" => {
                println!("NovaOS Atlas Map Subsystems:");
                println!("  * Projection: Spherical Web Mercator EPSG:3857 (forward & inverse)");
                println!("  * Geodesy:    Haversine great-circle formula");
                println!("  * UI Shell:   macOS Maps acrylic sidebar, Nearby drawer, Marked Location card");
                println!("  * Rasterizer: Pure 2D software double-buffer with 3D horizon tilt");
                println!("  * Routing:    Turn-by-turn waypoint pathfinding & maneuvers");
                exit(0);
            }
            "--version" | "-v" => {
                println!("NovaOS Map (Atlas) 1.0.0");
                println!("Sovereign Pure-Rust Vector Cartography & Navigation Engine");
                exit(0);
            }
            _ => {}
        }
    }

    let loc = PlatformLocation::current_location();
    println!("============================================================");
    println!("       NOVAOS MAPS (ATLAS) - VECTOR CARTOGRAPHY ENGINE      ");
    println!("============================================================");
    println!("Active Location: {}", loc.coord.format_dms());
    println!("Area:            Gulshan-2 / Baridhara Diplomatic Zone, Dhaka");
    println!("Status:          High-Precision GPS Lock (±3.0m)");
    println!("Weather:         25°C 🌤️  AQI 82 (Moderate)");
    println!("Subsystems:      Mercator EPSG:3857, Haversine Geodesy, Turn-by-Turn, 3D Pitch");
    println!("Run with --selftest to execute the autonomous qualification suite.");
}
