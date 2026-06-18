set windows-shell := ["powershell.exe"]
export RUST_BACKTRACE := "1"

# Displays the list of available commands
@just:
    just --list

# Installs the tools pinned in mise.toml (rust, wasm-bindgen, wasm-opt, trunk)
init:
    mise install

# Builds the Leptos page bundle into dist
build:
    trunk build

# Builds the page bundle and runs the desktop shell, which boots the host
run: build
    cargo run -p desktop

# Serves the page in the browser at http://127.0.0.1:8080 (observe-only, no host)
run-web:
    trunk serve

# Runs the headless two-node example against a host
example:
    cargo run -p mobius --example review_loop

# Produces a release page bundle in dist
dist:
    trunk build --release

# Builds the standalone executable with the page bundle embedded
build-desktop: dist
    cargo build --release -p desktop

# Runs cargo check across native and wasm targets
check:
    cargo check -p protocol -p mobius -p desktop
    cargo check -p protocol -p mobius-ui --target wasm32-unknown-unknown
    cargo fmt --all -- --check

# Runs clippy across native and wasm and denies warnings
lint:
    cargo clippy -p protocol -p mobius -p desktop -- -D warnings
    cargo clippy -p protocol -p mobius-ui --target wasm32-unknown-unknown -- -D warnings

# Formats the code
format:
    cargo fmt --all

# Removes build artifacts (Windows)
[windows]
clean:
    cargo clean
    Remove-Item -Recurse -Force dist -ErrorAction SilentlyContinue

# Removes build artifacts (Unix)
[unix]
clean:
    cargo clean
    rm -rf dist
