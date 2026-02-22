# Download reference implementations for inspiration and testing
inspirations:
    mkdir -p inspirations
    [ -d inspirations/supernote-tool ] || git clone https://github.com/jya-dev/supernote-tool.git inspirations/supernote-tool
    [ -d inspirations/pysn-digest ] || git clone https://gitlab.com/mmujynya/pysn-digest.git inspirations/pysn-digest
    [ -d inspirations/snlib] || git clone https://github.com/Walnut356/snlib.git inspirations/snlib
    echo "Inspirations downloaded to inspirations/"

# Run all tests
test:
    cargo test --workspace

# Lint and format check
check:
    cargo clippy --workspace -- -D warnings && cargo fmt --check

# Format code
fmt:
    cargo fmt

# Build in release mode
build:
    cargo build --workspace --release

# Run the CLI tool
run *args:
    cargo run -p calamus-cli -- {{args}}

# Build WASM module
wasm:
    wasm-pack build crates/wasm --target web --release

# Serve demo page locally (since using file:// doesn't allow wasm.)
serve: wasm
    python3 -m http.server 8080 --directory .

# Render all test notes to testoutput/ as PNG and SVG
testoutput: build
    mkdir -p testoutput
    for f in testfiles/*.note; do \
        name=$(basename "$f" .note); \
        target/release/calamus render "$f" "testoutput/${name}.png"; \
        target/release/calamus svg "$f" "testoutput/${name}.svg"; \
    done

# View a rendered page to compare between svg and png
view name page:
    imv testoutput/{{name}}_{{page}}.png testoutput/{{name}}_{{page}}.svg
