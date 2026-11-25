//! userland/desktop/apps/nova-map/src/lib.rs
//!
//! # NovaOS Map Application Library (nova-map / Atlas)
//!
//! Sovereign, pure-Rust default Vector Maps & Navigation Suite for NovaOS,
//! synthesizing macOS Maps (floating acrylic sidebar, Find Nearby drawer,
//! Marked Location card, weather telemetry) with Web Mercator projection,
//! geocoding, turn-by-turn routing, and 3D horizon pitch transform.

pub use atlas_core::{
    GeoCoord, MapDatabase, MapState, MercatorProjection, NavigationRoute, Poi, PoiCategory,
    RoadSegment, RouteStep, ScreenPoint,
};
pub use atlas_platform::{LocationTelemetry, PlatformLocation};
pub use atlas_render::{CanvasSurface, VectorMapRenderer};
pub use atlas_ui::{MapUiHitTarget, MapUiRenderer};

/// Autonomous qualification validation suite for nova-map.
pub fn run_nova_map_selftest() -> bool {
    // 1. Projection roundtrip test
    let coord = GeoCoord::new(23.80439, 90.41583);
    let (nx, ny) = MercatorProjection::geo_to_normalized(&coord);
    let back = MercatorProjection::normalized_to_geo(nx, ny);
    if (coord.lat - back.lat).abs() > 1e-4 || (coord.lon - back.lon).abs() > 1e-4 {
        eprintln!("[FAIL] Web Mercator roundtrip failed");
        return false;
    }

    // 2. Haversine distance
    let c2 = GeoCoord::new(23.79250, 90.41500);
    let dist = coord.distance_to(&c2);
    if dist < 1.0 || dist > 2.0 {
        eprintln!("[FAIL] Haversine distance out of bounds: {} km", dist);
        return false;
    }

    // 3. Spatial database loading
    let pois = MapDatabase::load_pois();
    let roads = MapDatabase::load_roads();
    let polys = MapDatabase::load_polygons();
    if pois.is_empty() || roads.is_empty() || polys.is_empty() {
        eprintln!("[FAIL] Cartographic database datasets missing");
        return false;
    }

    // 4. Geocoding search
    let matches = MapDatabase::search_pois("Hospital");
    if matches.is_empty() {
        eprintln!("[FAIL] Geocoding failed to match 'Hospital'");
        return false;
    }

    // 5. Turn-by-turn routing
    let mut state = MapState::default();
    state.plan_route(coord, c2, "United Hospital", "The Westin Dhaka");
    if state.active_route.is_none() {
        eprintln!("[FAIL] Routing calculation produced None");
        return false;
    }

    // 6. Vector rendering onto CanvasSurface
    let mut canvas = CanvasSurface::new(640, 480);
    VectorMapRenderer::render(&mut canvas, &state);
    let modified = canvas.buffer.iter().filter(|&&px| px != 0xFFF4F3F0).count();
    if modified < 500 {
        eprintln!("[FAIL] Canvas rendered fewer than 500 cartographic pixels: {}", modified);
        return false;
    }

    // 7. macOS Maps UI rendering
    MapUiRenderer::render(&mut canvas, &state);

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_map_qualification() {
        assert!(run_nova_map_selftest());
    }

    #[test]
    fn test_mercator_accuracy() {
        let center = GeoCoord::new(23.80439, 90.41583);
        let coord = GeoCoord::new(23.80500, 90.41600);
        let pt = MercatorProjection::geo_to_screen(&coord, &center, 15.0, 1024.0, 768.0);
        let back = MercatorProjection::screen_to_geo(pt, &center, 15.0, 1024.0, 768.0);
        assert!((coord.lat - back.lat).abs() < 0.001);
        assert!((coord.lon - back.lon).abs() < 0.001);
    }
}
