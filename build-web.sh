#!/bin/bash
set -e

echo "Building NexView for WebAssembly..."
wasm-pack build --target web --release

echo "Copying index.html to pkg/..."
cp web/index.html pkg/

echo ""
echo "Build complete! To run:"
echo "  cd pkg && python3 -m http.server 8080"
echo "  Then open http://localhost:8080 in your browser"
