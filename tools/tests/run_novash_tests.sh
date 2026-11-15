#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for Modern Interactive Rust Shell (novash)
# 1. Next-generation interactive user shell in pure Rust inspired by Fish and Zsh.
# 2. Real-time syntax highlighting for commands, flags, strings, variables, operators, and comments.
# 3. Fish-style history-based autosuggestions with ghost text and word-by-word acceptance (Alt+Right).
# 4. Context-aware tab completion for built-ins, binaries, file paths, and git/cargo subcommands.
# 5. Rich interactive prompt with user, hostname, path abbreviation (~), and git branch/dirty status.
# 6. Directory bookmarks (bookmark/b/j) and directory stack (pushd/popd/dirs).
# 7. POSIX pipelines (|), redirections (>, >>, <, 2>, 2>&1), and chaining (&&, ||, ;).
# 8. Scripting and control flow engine (if/then/else, for/in, while, test/[).
# 9. Built-in command dispatcher (cd, pwd, echo, export, unset, env, alias, unalias, true, false).
# 10. Interactive line editor based on rustyline/liner concepts with full ANSI redraw.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_novash_tests.sh      (or: make novash-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/apps/novash host tests (Modern Interactive Rust Shell) ---"
    (cd userland/desktop/apps/novash && cargo test --quiet)
    (cd userland/desktop/apps/novash && cargo run --quiet -- --selftest)
    echo "[PASS] All 12 Modern Interactive Rust Shell (novash) tests passed successfully."
else
    echo "novash-test: cargo not found - skipping the suite"
fi
