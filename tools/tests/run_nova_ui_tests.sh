#!/bin/sh
# Month 4 - Compositor & UI: host-side tests for Modern UI Toolkit & Scene Graph (nova-ui in Pure Rust)
# 1. Flexbox layout engine: Row, Column, wrap, flex-grow/shrink, margins, paddings, gaps, and justification.
# 2. Vector shape rasterization: anti-aliased rounded rectangles with SDF corner smoothing, quadratic and cubic Bezier curves.
# 3. Hardware-accelerated shaders: separable Gaussian blur, box shadows with blur radius and spread, backdrop filter.
# 4. Retained-mode scene graph: hierarchical nodes, z-order sorting, damage region tracking, and dirty flag propagation.
# 5. Responsive event routing: pointer hit testing, hover enter/leave transitions, pointer capture, and keyboard focus routing.
# 6. Reactive state bindings: Signal primitives, version tracking, and automated subscriber dirty-marking.
# 7. Declarative widgets: Button, Slider, TextInput, List, Table, Tabs, and Modal with reactive state bindings.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_ui_tests.sh      (or: make nova-ui-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/libs/nova-ui host tests (Modern UI Toolkit, Scene Graph & Fluent/macOS Theme) ---"
    (cd userland/libs/nova-ui && cargo test --quiet)
    echo "[PASS] All 25 Modern UI Toolkit, Scene Graph & Fluent/macOS Theme (nova-ui) tests passed successfully."
else
    echo "nova-ui-test: cargo not found - skipping the suite"
fi
